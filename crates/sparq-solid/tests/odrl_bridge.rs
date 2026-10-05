//! [OPUS-4.8] sq-h3uk — the ODRL→AUTH_GRAPH bridge: a definite ODRL Permit
//! materializes the equivalent WAC/ACP grant into `<urn:sparq:auth>` so the EXISTING
//! graph-level enforcement honours it; a Deny / ambiguous / unmapped eval
//! materializes NOTHING (fail-closed); the action→mode mapping is correct; and a
//! round-trip through the real enforcement path (`PodStore::accessible` /
//! `query_as`) grants exactly the intended access.
//!
//! Gated by the `odrl-bridge` feature (the whole test file no-ops without it).
#![cfg(feature = "odrl-bridge")]

use sparq_core::Graph;
use sparq_policy::{parse_policy_str, Request, Value};
use sparq_solid::{
    action_to_mode, materialize_permission, materialize_policy, materialize_prohibition, Mode,
    PodStore, Session,
};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const CAROL: &str = "https://carol.ex/card#me";
const N1: &str = "https://pod.ex/notes/n1";

fn odrl(local: &str) -> String {
    format!("{ODRL}{local}")
}

/// A pod with one content graph + no static ACL (so the only grants are bridged ones).
fn pod() -> Graph {
    Graph::load_dataset(
        "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/notes/n1> .",
        "nquads",
    )
    .expect("pod loads")
}

/// alice MAY read n1 (a bare matching permission, no constraints).
fn read_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/read> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

// ---------------------------------------------------------------------------
// 1. action → mode mapping correctness.
// ---------------------------------------------------------------------------
#[test]
fn action_mode_mapping() {
    assert_eq!(action_to_mode(&odrl("read")), Some(Mode::Read));
    assert_eq!(action_to_mode(&odrl("display")), Some(Mode::Read));
    assert_eq!(action_to_mode(&odrl("present")), Some(Mode::Read));
    assert_eq!(action_to_mode(&odrl("print")), Some(Mode::Read));
    assert_eq!(action_to_mode(&odrl("play")), Some(Mode::Read));
    assert_eq!(action_to_mode(&odrl("append")), Some(Mode::Append));
    assert_eq!(action_to_mode(&odrl("modify")), Some(Mode::Write));
    assert_eq!(action_to_mode(&odrl("delete")), Some(Mode::Write));
    assert_eq!(action_to_mode(&odrl("write")), Some(Mode::Write));
    // fail-closed: the umbrella + unknown + non-odrl actions are unmapped.
    assert_eq!(action_to_mode(&odrl("use")), None);
    assert_eq!(action_to_mode(&odrl("aggregate")), None);
    assert_eq!(action_to_mode("https://example.org/custom"), None);
}

// ---------------------------------------------------------------------------
// 2. Permit → materializes the expected grant triple in AUTH_GRAPH.
// ---------------------------------------------------------------------------
#[test]
fn permit_materializes_grant_triple() {
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission(&mut g, &read_policy(), &req);

    assert!(out.granted, "definite Permit should grant: {out:?}");
    assert_eq!(out.mode, Some(Mode::Read));
    assert_eq!(
        out.grant_triple,
        Some((ALICE.to_owned(), "https://sparq.dev/ns/auth#read".to_owned(), N1.to_owned())),
    );

    // The grant is a real triple in the <urn:sparq:auth> view, readable as such.
    let q = "SELECT ?who WHERE { GRAPH <urn:sparq:auth> { \
        ?who <https://sparq.dev/ns/auth#read> <https://pod.ex/notes/n1> } }";
    let rows = sparq_engine::query(&g, q).expect("query");
    assert_eq!(rows.rows.len(), 1, "exactly alice's read grant");
}

// ---------------------------------------------------------------------------
// 3. Round-trip through the REAL enforcement path (PodStore::accessible / query_as).
//    The bridged grant grants exactly the intended access, nothing more.
// ---------------------------------------------------------------------------
#[test]
fn round_trip_through_enforcement() {
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = store.materialize_odrl_permission(&read_policy(), &req);
    assert!(out.granted);

    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    // alice can READ n1 via the materialized grant…
    assert!(store.accessible(&alice, Mode::Read).iter().any(|gph| gph.as_str() == N1));
    // …but NOT write (the bridge only materialized a read grant — fail-closed)…
    assert!(store.accessible(&alice, Mode::Write).is_empty());

    // …and a DIFFERENT agent gets nothing (the grant is scoped to alice's WebID).
    let mallory = Session { agent: Some("https://mallory.ex/card#me"), client: None, issuer: None, now: None };
    assert!(store.accessible(&mallory, Mode::Read).is_empty());
    // anonymous likewise.
    assert!(store.accessible(&Session::default(), Mode::Read).is_empty());

    // End-to-end: alice's authorized query returns the content; others see nothing.
    // [OPUS-4.8] sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    assert_eq!(store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 1);
    assert_eq!(store.query_as(&mallory, Mode::Read, sel).unwrap().rows.len(), 0);
    assert_eq!(store.query_as(&Session::default(), Mode::Read, sel).unwrap().rows.len(), 0);
}

// ---------------------------------------------------------------------------
// 4. Deny → materializes NOTHING (fail-closed). Wrong party never matches.
// ---------------------------------------------------------------------------
#[test]
fn deny_materializes_nothing() {
    let mut store = PodStore::new(pod());
    // Mallory is not the assignee → no permission matches → DENY.
    let req = Request::new(odrl("read")).on(N1).by("https://mallory.ex/card#me");
    let out = store.materialize_odrl_permission(&read_policy(), &req);
    assert!(!out.granted, "deny must not grant: {out:?}");
    assert!(out.grant_triple.is_none());

    // Nobody gains access — the auth view holds no bridged grant.
    let mallory = Session { agent: Some("https://mallory.ex/card#me"), client: None, issuer: None, now: None };
    assert!(store.accessible(&mallory, Mode::Read).is_empty());
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Read).is_empty());
}

// ---------------------------------------------------------------------------
// 5. Ambiguous / unsatisfied-constraint eval → NOTHING (fail-closed).
//    A time-windowed permission with NO dateTime context fails the constraint.
// ---------------------------------------------------------------------------
#[test]
fn unsatisfied_constraint_materializes_nothing() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/win> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ;
                      odrl:operator odrl:lteq ;
                      odrl:rightOperand "2020-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ] ] .
"#,
        "turtle",
    )
    .unwrap();

    let mut store = PodStore::new(pod());
    // Out-of-window request (after the bound) → constraint unsatisfied → DENY → nothing.
    let req = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .with(odrl("dateTime"), Value::DateTime("2026-06-16T00:00:00Z".to_owned()));
    let out = store.materialize_odrl_permission(&pol, &req);
    assert!(!out.granted, "out-of-window must not grant: {out:?}");
    assert!(store.accessible(&Session { agent: Some(ALICE), client: None, issuer: None, now: None }, Mode::Read).is_empty());

    // And the SAME policy with NO dateTime evidence also fails closed.
    let mut store2 = PodStore::new(pod());
    let req2 = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(!store2.materialize_odrl_permission(&pol, &req2).granted);
}

// ---------------------------------------------------------------------------
// 6. Unmapped action (the umbrella) → Permit but NO grant (fail-closed).
//    `odrl:use` PERMITS a `use` request in the evaluator, yet `use` has no faithful
//    single WAC mode, so the bridge must materialize nothing.
// ---------------------------------------------------------------------------
#[test]
fn unmapped_action_materializes_nothing() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/use> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();

    let mut g = pod();
    // The request action is `odrl:use` itself — a definite Permit, but unmapped.
    let req = Request::new(odrl("use")).on(N1).by(ALICE);
    let out = materialize_permission(&mut g, &pol, &req);
    assert!(!out.granted, "umbrella action has no mode mapping: {out:?}");
    assert!(out.grant_triple.is_none());
    assert!(!out.reasons.is_empty());

    // But a CONCRETE `read` request against the same `use` permission DOES grant
    // (the evaluator's umbrella permits it; the bridge maps the concrete request).
    let mut g2 = pod();
    let req2 = Request::new(odrl("read")).on(N1).by(ALICE);
    let out2 = materialize_permission(&mut g2, &pol, &req2);
    assert!(out2.granted, "use permission + concrete read request grants: {out2:?}");
    assert_eq!(out2.mode, Some(Mode::Read));
}

// ---------------------------------------------------------------------------
// 6a. SPARQL-query action contract: query uses `odrl:read`, never `odrl:use`.
//     [SONNET-4.6] sq-lrtc3.2.
// ---------------------------------------------------------------------------
#[test]
fn sparql_query_requires_concrete_read_action() {
    let use_policy = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/query-use> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";

    // `odrl:use` is an umbrella, not the bridge's SPARQL-query action. Even though
    // the policy evaluator permits the use request, its action has no single WAC
    // mode, so no grant reaches the real query path.
    let mut use_store = PodStore::new(pod());
    let use_request = Request::new(odrl("use")).on(N1).by(ALICE);
    let use_outcome = use_store.materialize_odrl_permission(&use_policy, &use_request);
    assert!(!use_outcome.granted, "odrl:use must stay unmapped: {use_outcome:?}");
    assert!(
        use_outcome.reasons.iter().any(|reason| reason.contains("no WAC/ACP mode mapping")),
        "expected an unmapped-action refusal, not a policy denial: {use_outcome:?}"
    );
    assert!(use_outcome.mode.is_none());
    assert!(use_outcome.grant_triple.is_none());
    assert_eq!(use_store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 0);

    // A query is represented by the concrete `odrl:read` action, which maps to
    // exactly Mode::Read and exposes the target through query_as.
    let mut read_store = PodStore::new(pod());
    let read_request = Request::new(odrl("read")).on(N1).by(ALICE);
    let read_outcome = read_store.materialize_odrl_permission(&read_policy(), &read_request);
    assert!(read_outcome.granted, "odrl:read should grant query access: {read_outcome:?}");
    assert_eq!(read_outcome.mode, Some(Mode::Read));
    assert_eq!(read_store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 1);
}

// ---------------------------------------------------------------------------
// 7. Partyless / targetless Permit → NOTHING (a partyless grant would widen access).
// ---------------------------------------------------------------------------
#[test]
fn partyless_or_targetless_materializes_nothing() {
    // An unrefined permission (no assignee, no target) matches an anonymous request
    // — but a grant with no concrete principal would widen access to everyone.
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/any> a odrl:Set ; odrl:permission [ odrl:action odrl:read ] .
"#,
        "turtle",
    )
    .unwrap();

    // No party.
    let mut g = pod();
    let no_party = Request::new(odrl("read")).on(N1);
    assert!(!materialize_permission(&mut g, &pol, &no_party).granted);

    // Party but no target.
    let mut g2 = pod();
    let no_target = Request::new(odrl("read")).by(ALICE);
    assert!(!materialize_permission(&mut g2, &pol, &no_target).granted);
}

// ---------------------------------------------------------------------------
// 8. The bridge APPENDS to an existing WAC view without clobbering static grants.
// ---------------------------------------------------------------------------
#[test]
fn bridge_preserves_existing_wac_grants() {
    // A pod whose static .acl grants BOB read on n1.
    let nq = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n1.acl> .
"#;
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
    store.materialize_wac().expect("wac materializes");

    // Does `agent` have read on n1 through the store's current enforcement view?
    fn reads_n1(s: &mut PodStore, agent: &str) -> bool {
        let sess = Session { agent: Some(agent), client: None, issuer: None, now: None };
        s.accessible(&sess, Mode::Read).iter().any(|g| g.as_str() == N1)
    }

    assert!(reads_n1(&mut store, "https://bob.ex/card#me"), "bob's static grant");

    // Now bridge an ODRL read grant for alice on the same graph.
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission(&read_policy(), &req).granted);

    // BOTH grants hold: bob (static WAC) AND alice (bridged ODRL).
    assert!(reads_n1(&mut store, "https://bob.ex/card#me"), "bob preserved");
    assert!(reads_n1(&mut store, ALICE), "alice bridged");
}

// ===========================================================================
// [OPUS-4.8] sq-w693 — Prohibition → explicit auth:deny<Mode> (deny-overrides).
// ===========================================================================

/// alice is PROHIBITED from writing n1 (a bare matching prohibition, no constraints).
fn write_prohibition() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/prohib> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:modify ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

// ---------------------------------------------------------------------------
// 9. A matched Prohibition materializes the expected auth:deny<Mode> triple.
// ---------------------------------------------------------------------------
#[test]
fn prohibition_materializes_deny_triple() {
    let mut g = pod();
    let req = Request::new(odrl("modify")).on(N1).by(ALICE);
    let out = materialize_prohibition(&mut g, &write_prohibition(), &req);

    assert!(out.prohibited, "matched Prohibition should deny: {out:?}");
    assert!(!out.granted, "a prohibition is not a grant");
    assert_eq!(out.mode, Some(Mode::Write));
    assert_eq!(
        out.deny_triple,
        Some((ALICE.to_owned(), "https://sparq.dev/ns/auth#denyWrite".to_owned(), N1.to_owned())),
    );

    // The deny is a real triple in the <urn:sparq:auth> view, readable as such.
    let q = "SELECT ?who WHERE { GRAPH <urn:sparq:auth> { \
        ?who <https://sparq.dev/ns/auth#denyWrite> <https://pod.ex/notes/n1> } }";
    let rows = sparq_engine::query(&g, q).expect("query");
    assert_eq!(rows.rows.len(), 1, "exactly alice's denyWrite");
}

// ---------------------------------------------------------------------------
// 10. DENY-OVERRIDES through the REAL enforcement path: a principal with BOTH an
//     allow grant AND a deny for the same mode is DENIED (deny beats allow).
// ---------------------------------------------------------------------------
#[test]
fn deny_overrides_allow_through_enforcement() {
    let mut store = PodStore::new(pod());

    // First grant alice WRITE on n1 (an ODRL Permit → auth:write).
    let permit = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/w> a odrl:Set ; odrl:permission [
    odrl:action odrl:modify ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();
    let wreq = Request::new(odrl("modify")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission(&permit, &wreq).granted);

    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    // Sanity: the allow grant is live through the real enforcement path.
    assert!(
        store.accessible(&alice, Mode::Write).iter().any(|g| g.as_str() == N1),
        "alice can write n1 BEFORE the deny is materialized",
    );

    // Now materialize the matching Prohibition (auth:denyWrite for the same mode).
    let dreq = Request::new(odrl("modify")).on(N1).by(ALICE);
    let out = store.materialize_odrl_prohibition(&write_prohibition(), &dreq);
    assert!(out.prohibited, "deny materialized: {out:?}");

    // DENY-OVERRIDES: alice is now DENIED write through the real enforcement path,
    // even though the allow grant is still present in the auth view.
    assert!(
        store.accessible(&alice, Mode::Write).is_empty(),
        "deny beats allow: alice can no longer write n1",
    );
    // And the write-path update enforcement honours it too (fail-closed).
    let ins = "INSERT DATA { GRAPH <https://pod.ex/notes/n1> { \
        <https://pod.ex/notes/n1#it> <https://ex.dev/ns#tag> \"x\" } }";
    assert!(store.update_as(&alice, ins).is_err(), "denied write update fails closed");
}

// ---------------------------------------------------------------------------
// 11. A single policy with BOTH a permission and a prohibition on the same
//     principal/target/mode → materialize both → DENIED (deny wins).
// ---------------------------------------------------------------------------
#[test]
fn permit_plus_prohibition_same_subject_is_denied() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/both> a odrl:Set ;
    odrl:permission [
        odrl:action odrl:modify ;
        odrl:target <https://pod.ex/notes/n1> ;
        odrl:assignee <https://alice.ex/card#me> ] ;
    odrl:prohibition [
        odrl:action odrl:modify ;
        odrl:target <https://pod.ex/notes/n1> ;
        odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();

    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("modify")).on(N1).by(ALICE);
    let out = store.materialize_odrl_policy(&pol, &req);

    // The prohibition side materializes a deny. The permit side does NOT: the ODRL
    // evaluator ALREADY applies deny-overrides (a matching prohibition overrides any
    // permission), so `evaluate(...).allow == false` and no allow grant is emitted —
    // deny-overrides holds even more strongly (the allow is never written at all).
    assert!(!out.granted, "the permit is overridden by the prohibition (evaluator): {out:?}");
    assert!(out.prohibited, "the prohibition side materialized: {out:?}");
    assert_eq!(out.mode, Some(Mode::Write), "deny mode is operative under deny-overrides");
    assert!(out.deny_triple.is_some());

    // Net effect through the real enforcement: alice is DENIED.
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(
        store.accessible(&alice, Mode::Write).is_empty(),
        "deny-overrides: a permission + prohibition on the same subject denies",
    );
}

// ---------------------------------------------------------------------------
// 12. No matching prohibition (wrong party / different mode / unmapped / partyless /
//     targetless) → materialize NOTHING (fail-closed; access never silently widened).
// ---------------------------------------------------------------------------
#[test]
fn unmatched_prohibition_materializes_nothing() {
    // (a) Wrong party — the prohibition names alice; mallory isn't carved out.
    let mut g = pod();
    let wrong_party = Request::new(odrl("modify")).on(N1).by("https://mallory.ex/card#me");
    let out = materialize_prohibition(&mut g, &write_prohibition(), &wrong_party);
    assert!(!out.prohibited, "wrong party is not carved out: {out:?}");
    assert!(out.deny_triple.is_none());

    // (b) Different action/mode — the prohibition forbids modify (Write); a read
    //     request matches no prohibition, so no deny is materialized.
    let mut g2 = pod();
    let read_req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(!materialize_prohibition(&mut g2, &write_prohibition(), &read_req).prohibited);

    // (c) Unmapped action — a prohibition on the `use` umbrella matches a `use`
    //     request, but `use` has no faithful single mode → materialize nothing, and
    //     SAY SO (a dropped deny would widen access).
    let use_prohib = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/u> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:use ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();
    let mut g3 = pod();
    let use_req = Request::new(odrl("use")).on(N1).by(ALICE);
    let out3 = materialize_prohibition(&mut g3, &use_prohib, &use_req);
    assert!(!out3.prohibited, "unmapped umbrella deny not materialized: {out3:?}");
    assert!(!out3.reasons.is_empty(), "the unmappable carve-out is reported, not silent");

    // (d) Partyless prohibition (no assignee) matched by a partyless request → nothing
    //     (a deny with no concrete principal is meaningless here).
    let any_prohib = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/anyp> a odrl:Set ; odrl:prohibition [ odrl:action odrl:modify ] .
"#,
        "turtle",
    )
    .unwrap();
    let mut g4 = pod();
    let no_party = Request::new(odrl("modify")).on(N1);
    assert!(!materialize_prohibition(&mut g4, &any_prohib, &no_party).prohibited);

    // (e) Targetless request → nothing.
    let mut g5 = pod();
    let no_target = Request::new(odrl("modify")).by(ALICE);
    assert!(!materialize_prohibition(&mut g5, &any_prohib, &no_target).prohibited);
}

// ---------------------------------------------------------------------------
// 13. Regression: the Permit-only path still works unchanged via materialize_policy
//     (a policy with no prohibition grants exactly as before, no deny emitted).
// ---------------------------------------------------------------------------
#[test]
fn permit_only_regression_via_policy() {
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_policy(&mut g, &read_policy(), &req);

    assert!(out.granted, "permit-only still grants: {out:?}");
    assert!(!out.prohibited, "no prohibition → no deny");
    assert_eq!(out.mode, Some(Mode::Read));
    assert!(out.deny_triple.is_none());
    assert!(out.grant_triple.is_some());

    // End-to-end through the enforcement path: alice reads, deny absent.
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_policy(&read_policy(), &req).granted);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Read).iter().any(|g| g.as_str() == N1));
}

// ===========================================================================
// [OPUS-4.8] sq-hiz4 — conditional grants: a FAITHFULLY-mappable constraint
// (recipient/assignee → agent matcher) persists as an `auth:ConditionalGrant`
// and is RE-CHECKED per session; an UNmappable constraint stays one-shot.
// ===========================================================================
use sparq_solid::materialize_permission_conditional;

fn reads(store: &mut PodStore, agent: &str) -> bool {
    let s = Session { agent: Some(agent), client: None, issuer: None, now: None };
    store.accessible(&s, Mode::Read).iter().any(|g| g.as_str() == N1)
}

/// Count `auth:ConditionalGrant` heads naming `agent` (or any, if `agent` is None) in
/// the materialized auth view of a freshly-bridged `graph`.
fn cond_grants_for(graph: &Graph, agent: Option<&str>) -> usize {
    let q = match agent {
        Some(a) => format!(
            "SELECT ?g WHERE {{ GRAPH <urn:sparq:auth> {{ \
             ?g <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
                <https://sparq.dev/ns/auth#ConditionalGrant> ; \
                <https://sparq.dev/ns/auth#agent> <{a}> }} }}"
        ),
        None => "SELECT ?g WHERE { GRAPH <urn:sparq:auth> { \
             ?g <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
                <https://sparq.dev/ns/auth#ConditionalGrant> } }"
            .to_owned(),
    };
    sparq_engine::query(graph, &q).expect("query").rows.len()
}

/// A permission whose RECIPIENT is constrained to carol (not whoever materialized it).
fn recipient_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/recip> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ;
                      odrl:operator odrl:eq ;
                      odrl:rightOperand <https://carol.ex/card#me> ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

// 14. A recipient constraint persists as a ConditionalGrant re-checked per session:
//     carol (the recipient) is granted; everyone else is denied — through accessible/
//     query_as — even though ALICE materialized it.
#[test]
fn recipient_constraint_persists_as_rechecked_condition() {
    // Inspect the materialized triples via the free-function form (we own the Graph).
    // Note: the materializing request party is ALICE, but the constraint names CAROL.
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &recipient_policy(), &req);
    assert!(out.granted, "faithful recipient maps to a condition: {out:?}");
    // A real ConditionalGrant head naming carol now lives in the auth view.
    assert_eq!(cond_grants_for(&g, Some(CAROL)), 1, "carol condition present");
    assert_eq!(cond_grants_for(&g, Some(ALICE)), 0, "no condition for the materializer");

    // RE-CHECK through the real enforcement path: rebuild a store over the SAME graph.
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&recipient_policy(), &req).granted);
    assert!(reads(&mut store, CAROL), "recipient carol granted");
    assert!(!reads(&mut store, ALICE), "materializing party is NOT auto-granted");
    assert!(!reads(&mut store, BOB), "unrelated agent denied");
    assert!(!reads(&mut store, "https://x.ex/#m"), "stranger denied");
    assert!(store.accessible(&Session::default(), Mode::Read).is_empty(), "anonymous denied");

    // End-to-end through query_as: only carol sees the content.
    // [OPUS-4.8] sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let carol = Session { agent: Some(CAROL), client: None, issuer: None, now: None };
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert_eq!(store.query_as(&carol, Mode::Read, sel).unwrap().rows.len(), 1);
    assert_eq!(store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 0);
}

// 15. A recipient `isPartOf` SET → one re-checked condition per member (OR).
#[test]
fn recipient_set_persists_as_multiple_conditions() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/set> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ;
                      odrl:operator odrl:isPartOf ;
                      odrl:rightOperand "https://bob.ex/card#me|https://carol.ex/card#me" ] ] .
"#,
        "turtle",
    )
    .unwrap();
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);

    assert!(reads(&mut store, BOB), "bob in set");
    assert!(reads(&mut store, CAROL), "carol in set");
    assert!(!reads(&mut store, ALICE), "alice not in set");
    assert!(!reads(&mut store, "https://dave.ex/#m"), "dave not in set");
}

// 16. An UNmappable constraint (`odrl:purpose`) STAYS one-shot: the persisted view
//     holds NO ConditionalGrant; access is the frozen check against the supplied
//     request context (granted only if purpose matched at materialization), scoped
//     to the materializing party.
#[test]
fn purpose_constraint_stays_one_shot() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/purp> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ;
                      odrl:operator odrl:eq ;
                      odrl:rightOperand "research" ] ] .
"#,
        "turtle",
    )
    .unwrap();

    // (a) purpose SATISFIED at materialization → one-shot grant (frozen) for alice.
    let req = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .with(odrl("purpose"), Value::Str("research".to_owned()));
    let mut g = pod();
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(out.granted, "purpose satisfied → one-shot grant: {out:?}");
    // NO ConditionalGrant was persisted (purpose has no faithful ACP analogue).
    assert_eq!(cond_grants_for(&g, None), 0, "purpose must NOT become a re-checked condition");

    // The frozen grant is scoped to the materializing party (alice).
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(reads(&mut store, ALICE), "alice one-shot grant");
    assert!(!reads(&mut store, CAROL), "no widening");

    // (b) purpose NOT satisfied (missing context) → fail-closed, nothing granted.
    let mut store2 = PodStore::new(pod());
    let bad = Request::new(odrl("read")).on(N1).by(ALICE); // no purpose context
    assert!(!store2.materialize_odrl_permission_conditional(&pol, &bad).granted);
    assert!(!reads(&mut store2, ALICE), "unsatisfied purpose grants nothing");
}

// 17a. MIXED constraints with a STRICT dateTime bound (`lt`) fail SAFE: the strict
//     bound has no inclusive auth:notBefore/notAfter analogue, so the WHOLE rule stays
//     one-shot (frozen) and a persisted recipient-only condition that LOST the bound is
//     never emitted (over-grant). [OPUS-4.8] sq-0q7n — strict bounds stay Unmappable.
#[test]
fn mixed_mappable_and_strict_datetime_stays_one_shot() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/mix> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ;
                      odrl:operator odrl:eq ;
                      odrl:rightOperand <https://carol.ex/card#me> ] ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ;
                      odrl:operator odrl:lt ;
                      odrl:rightOperand "2020-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ] ] .
"#,
        "turtle",
    )
    .unwrap();

    // Out-of-window request → DENY → nothing (the strict time bound is NOT dropped).
    let req = Request::new(odrl("read"))
        .on(N1)
        .by(CAROL)
        .with(odrl("dateTime"), Value::DateTime("2026-06-16T00:00:00Z".to_owned()));
    let mut g = pod();
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(!out.granted, "out-of-window with a strict dateTime bound must NOT grant: {out:?}");
    // No ConditionalGrant leaked carol an unconditional re-checked allow.
    assert_eq!(cond_grants_for(&g, None), 0, "no condition emitted when the bound is unmappable");
    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(!reads(&mut store, CAROL), "no over-grant from dropping the strict time bound");
}

// 17b. [OPUS-4.8] sq-0q7n — recipient (mappable) + dateTime `lteq` (now ALSO mappable):
//     the rule persists ONE recipient ConditionalGrant carrying an `auth:notAfter`
//     window, re-checked against the LIVE clock per request — NOT frozen at
//     materialization. carol inside the window reads; carol after it is denied WITHOUT
//     a ledger refresh; the materializing request's `odrl:dateTime` is irrelevant.
#[test]
fn recipient_with_datetime_window_rechecks_live_clock() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/win> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ;
                      odrl:operator odrl:eq ;
                      odrl:rightOperand <https://carol.ex/card#me> ] ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ;
                      odrl:operator odrl:lteq ;
                      odrl:rightOperand "2026-12-31T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ] ] .
"#,
        "turtle",
    )
    .unwrap();

    // Materialize WITHOUT any request dateTime — the window is persisted, not checked now.
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let mut g = pod();
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(out.granted, "recipient+window maps to a re-checked condition: {out:?}");
    // Exactly ONE ConditionalGrant for carol, carrying an auth:notAfter bound.
    assert_eq!(cond_grants_for(&g, Some(CAROL)), 1, "one windowed condition for carol");
    let na = sparq_engine::query(
        &g,
        "SELECT ?t WHERE { GRAPH <urn:sparq:auth> { \
         ?g <https://sparq.dev/ns/auth#agent> <https://carol.ex/card#me> ; \
            <https://sparq.dev/ns/auth#notAfter> ?t } }",
    )
    .expect("query")
    .rows
    .len();
    assert_eq!(na, 1, "the grant carries an auth:notAfter window bound");

    // RE-CHECK the LIVE clock through the real enforcement path.
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);
    let carol_in = Session { agent: Some(CAROL), client: None, issuer: None, now: None }
        .at("2026-06-17T00:00:00Z"); // inside [.., 2026-12-31]
    let carol_after = Session { agent: Some(CAROL), client: None, issuer: None, now: None }
        .at("2027-01-01T00:00:00Z"); // AFTER the window closed
    let carol_noclock = Session { agent: Some(CAROL), client: None, issuer: None, now: None };
    assert!(
        store.accessible(&carol_in, Mode::Read).iter().any(|x| x.as_str() == N1),
        "carol inside the window reads"
    );
    assert!(
        store.accessible(&carol_after, Mode::Read).iter().all(|x| x.as_str() != N1),
        "carol AFTER the window is denied — live-clock re-check, no refresh needed"
    );
    assert!(
        store.accessible(&carol_noclock, Mode::Read).iter().all(|x| x.as_str() != N1),
        "a windowed grant with NO clock fails closed"
    );
    // The window is recipient-scoped: bob (wrong recipient) is denied even inside it.
    let bob_in = Session { agent: Some(BOB), client: None, issuer: None, now: None }
        .at("2026-06-17T00:00:00Z");
    assert!(
        store.accessible(&bob_in, Mode::Read).iter().all(|x| x.as_str() != N1),
        "non-recipient denied even inside the window"
    );
}

// 17c. [OPUS-4.8] sq-0q7n — a PUBLIC (no recipient) two-sided window: `gteq` lower +
//     `lteq` upper map to auth:notBefore + auth:notAfter on a public ConditionalGrant.
//     Any session inside the window reads; before/after — and a clockless session —
//     are denied through the live re-check.
#[test]
fn public_two_sided_datetime_window_rechecks_live_clock() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/two> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gteq ;
                      odrl:rightOperand "2026-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ] ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
                      odrl:rightOperand "2026-12-31T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ] ] .
"#,
        "turtle",
    )
    .unwrap();
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);

    let inside = Session { agent: Some(BOB), client: None, issuer: None, now: None }
        .at("2026-06-17T00:00:00Z");
    let before = Session { agent: Some(BOB), client: None, issuer: None, now: None }
        .at("2025-06-01T00:00:00Z");
    let after = Session { agent: Some(BOB), client: None, issuer: None, now: None }
        .at("2027-06-01T00:00:00Z");
    let no_clock = Session { agent: Some(BOB), client: None, issuer: None, now: None };
    assert!(store.accessible(&inside, Mode::Read).iter().any(|x| x.as_str() == N1), "inside window");
    assert!(store.accessible(&before, Mode::Read).iter().all(|x| x.as_str() != N1), "before window");
    assert!(store.accessible(&after, Mode::Read).iter().all(|x| x.as_str() != N1), "after window");
    assert!(store.accessible(&no_clock, Mode::Read).iter().all(|x| x.as_str() != N1), "no clock fails closed");
}

// 18. Compose-with-deny: a matching prohibition overrides the conditional path
//     (deny-overrides) — nothing is materialized.
#[test]
fn prohibition_overrides_conditional_path() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/po> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
                      odrl:rightOperand <https://carol.ex/card#me> ] ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] .
"#,
        "turtle",
    )
    .unwrap();
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = store.materialize_odrl_permission_conditional(&pol, &req);
    assert!(!out.granted, "prohibition overrides: {out:?}");
    assert!(!reads(&mut store, CAROL), "deny-overrides: carol gets nothing");
}

// 19. A bare permission with NO constraints, via the conditional entry point, grants
//     PUBLIC (any session) — only valid because action/target/duties already held.
//     (Documents the no-constraint → auth:Public head behaviour.)
#[test]
fn no_constraint_conditional_grants_public() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/pub> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] .
"#,
        "turtle",
    )
    .unwrap();
    // Free-function form: a PUBLIC ConditionalGrant head is materialized.
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(out.granted, "bare permission grants: {out:?}");
    assert_eq!(cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")), 1);

    // Through the real enforcement path: public read = any agent AND anonymous.
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(reads(&mut store, BOB));
    assert!(!store.accessible(&Session::default(), Mode::Read).is_empty(), "anon public read");
}

// ===========================================================================
// [OPUS-4.8] sq-dpk4 — refresh / REVOCATION of bridged ODRL grants when the
// underlying ODRL policy changes. SECURITY-SENSITIVE: a withdrawn permission, a
// lapsed time window, or a re-evaluation that now Denies must LOSE access; a static
// WAC/ACP grant must NEVER be dropped; a still-valid bridged grant must survive.
// All assertions go through the REAL enforcement path (accessible / query_as).
// ===========================================================================
use sparq_solid::BridgeKind;

/// alice MAY read n1 ONLY until 2026-01-01 (a time-windowed permission).
fn windowed_read_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/win> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
                      odrl:rightOperand "2026-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

/// A policy that grants NOTHING (the permission has been WITHDRAWN entirely).
fn empty_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> . <urn:pol/read> a odrl:Set ."#,
        "turtle",
    )
    .expect("policy parses")
}

// 20. ADVERSARIAL "stale grant loses access": a bridged grant is materialized →
//     access granted; the ODRL policy then WITHDRAWS the permission → refresh →
//     access is GONE through the real enforcement path. We adversarially check the
//     grant did not survive in ANY form (accessible, query_as, the raw auth view, the
//     provenance graph) and that re-refreshing cannot resurrect it.
#[test]
fn withdrawn_permission_loses_access_after_refresh() {
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);

    // Materialize → alice has read.
    assert!(store.materialize_odrl_permission(&read_policy(), &req).granted);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(
        store.accessible(&alice, Mode::Read).iter().any(|g| g.as_str() == N1),
        "bridged grant is live before withdrawal",
    );
    // [OPUS-4.8] sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    assert_eq!(store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 1);

    // The policy WITHDRAWS the permission → refresh against the new (empty) policy.
    let (matched, retracted) =
        store.refresh_odrl_grant(&empty_policy(), &req, BridgeKind::Permission);
    assert!(matched, "the tracked grant slot matched");
    assert_eq!(retracted, 1, "the withdrawn grant was retracted");

    // ADVERSARIAL: access is GONE through every observable surface.
    assert!(
        store.accessible(&alice, Mode::Read).is_empty(),
        "STALE GRANT MUST LOSE ACCESS: alice can no longer read n1",
    );
    assert_eq!(
        store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(),
        0,
        "query_as returns nothing after revocation",
    );
    // The raw auth view holds no residual bridged grant for alice…
    let leftover = "SELECT ?p ?o WHERE { GRAPH <urn:sparq:auth> { \
        <https://alice.ex/card#me> ?p ?o } }";
    assert_eq!(sparq_engine::query(&store.graph, leftover).unwrap().rows.len(), 0,
        "no residual alice triple in the enforcement view");
    // …and the provenance graph was cleared of it.
    let prov = "SELECT ?s ?p ?o WHERE { GRAPH <urn:sparq:auth-bridged> { ?s ?p ?o } }";
    assert_eq!(sparq_engine::query(&store.graph, prov).unwrap().rows.len(), 0,
        "no residual provenance after retraction");
    // Re-refreshing cannot resurrect a dropped grant (the ledger is empty now).
    assert_eq!(store.refresh_odrl_grants(), 0, "nothing left to retract");
    assert!(store.accessible(&alice, Mode::Read).is_empty(), "stays revoked");
}

// 21. LAPSED TIME WINDOW: a windowed grant valid at materialization-time loses access
//     once the window lapses (re-evaluate with a NOW past the bound) → refresh → gone.
#[test]
fn lapsed_time_window_loses_access_after_refresh() {
    let mut store = PodStore::new(pod());
    let pol = windowed_read_policy();

    // In-window request → granted.
    let in_window = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .with(odrl("dateTime"), Value::DateTime("2025-06-01T00:00:00Z".to_owned()));
    assert!(store.materialize_odrl_permission(&pol, &in_window).granted);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Read).iter().any(|g| g.as_str() == N1));

    // The window LAPSES: re-evaluate with a NOW past the bound (same policy, new ctx).
    let now_past = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .with(odrl("dateTime"), Value::DateTime("2026-06-16T00:00:00Z".to_owned()));
    let (matched, retracted) =
        store.refresh_odrl_grant(&pol, &now_past, BridgeKind::Permission);
    assert!(matched);
    assert_eq!(retracted, 1, "lapsed window retracts the grant");
    assert!(
        store.accessible(&alice, Mode::Read).is_empty(),
        "lapsed time window: access gone",
    );
}

// 22. RE-EVAL NOW DENIES (a prohibition is added): a bridged write grant is revoked
//     when the refreshed policy now carries a matching prohibition (deny-overrides).
#[test]
fn reeval_now_denies_loses_access_after_refresh() {
    let mut store = PodStore::new(pod());
    let permit = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/w> a odrl:Set ; odrl:permission [
    odrl:action odrl:modify ; odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();
    let req = Request::new(odrl("modify")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_policy(&permit, &req).granted);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Write).iter().any(|g| g.as_str() == N1));

    // The policy now ADDS a prohibition on the same action → re-eval Denies.
    let now_prohibited = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/w> a odrl:Set ;
    odrl:permission [ odrl:action odrl:modify ; odrl:target <https://pod.ex/notes/n1> ;
        odrl:assignee <https://alice.ex/card#me> ] ;
    odrl:prohibition [ odrl:action odrl:modify ; odrl:target <https://pod.ex/notes/n1> ;
        odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();
    let (matched, _) = store.refresh_odrl_grant(&now_prohibited, &req, BridgeKind::Policy);
    assert!(matched);
    // deny-overrides: even though materialize_policy emits a deny on replay, the net
    // enforcement result is DENIED (the allow is overridden upstream / by the deny).
    assert!(
        store.accessible(&alice, Mode::Write).is_empty(),
        "re-eval now Denies: alice can no longer write n1",
    );
}

// 23. A STILL-VALID bridged grant SURVIVES refresh (no spurious retraction).
#[test]
fn valid_bridged_grant_survives_refresh() {
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission(&read_policy(), &req).granted);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Read).iter().any(|g| g.as_str() == N1));

    // Plain refresh (policy unchanged) re-evaluates and KEEPS the still-valid grant.
    assert_eq!(store.refresh_odrl_grants(), 0, "valid grant not retracted");
    assert!(
        store.accessible(&alice, Mode::Read).iter().any(|g| g.as_str() == N1),
        "still-valid bridged grant survives refresh",
    );
    // Refresh against the SAME policy also keeps it.
    let (matched, retracted) =
        store.refresh_odrl_grant(&read_policy(), &req, BridgeKind::Permission);
    assert!(matched);
    assert_eq!(retracted, 0);
    assert!(store.accessible(&alice, Mode::Read).iter().any(|g| g.as_str() == N1));
}

// 24. A STATIC WAC grant is NOT dropped by a bridged-grant refresh (provenance keeps
//     static and bridged apart). bob (static) keeps read; alice (bridged, then revoked)
//     loses it — in the SAME store, through the SAME enforcement path.
#[test]
fn static_grant_not_dropped_by_refresh() {
    // bob's static .acl grant + a content graph.
    let nq = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n1.acl> .
"#;
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
    store.materialize_wac().expect("wac materializes");

    fn reads_n1(s: &mut PodStore, agent: &str) -> bool {
        let sess = Session { agent: Some(agent), client: None, issuer: None, now: None };
        s.accessible(&sess, Mode::Read).iter().any(|g| g.as_str() == N1)
    }

    // Bridge alice on top of bob's static grant.
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission(&read_policy(), &req).granted);
    assert!(reads_n1(&mut store, BOB), "bob static before");
    assert!(reads_n1(&mut store, ALICE), "alice bridged before");

    // Revoke alice's bridged grant. bob's STATIC grant must be untouched.
    let (matched, retracted) =
        store.refresh_odrl_grant(&empty_policy(), &req, BridgeKind::Permission);
    assert!(matched);
    assert_eq!(retracted, 1);
    assert!(reads_n1(&mut store, BOB), "STATIC GRANT PRESERVED: bob still reads n1");
    assert!(!reads_n1(&mut store, ALICE), "bridged grant revoked");
}

// 25. PROVENANCE distinguishes bridged vs static: a static grant never appears in the
//     bridged-provenance graph; a bridged grant does (and exactly the auth triple it
//     emitted).
#[test]
fn provenance_distinguishes_bridged_from_static() {
    let nq = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n1.acl> .
"#;
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
    store.materialize_wac().expect("wac materializes");
    // After a pure static materialize, the provenance graph is empty.
    let prov_all = "SELECT ?s ?p ?o WHERE { GRAPH <urn:sparq:auth-bridged> { ?s ?p ?o } }";
    assert_eq!(sparq_engine::query(&store.graph, prov_all).unwrap().rows.len(), 0,
        "no provenance for static grants");

    // Bridge alice → exactly her grant triple appears in provenance, bob's does not.
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission(&read_policy(), &req).granted);
    let alice_prov = "SELECT ?p WHERE { GRAPH <urn:sparq:auth-bridged> { \
        <https://alice.ex/card#me> <https://sparq.dev/ns/auth#read> <https://pod.ex/notes/n1> } }";
    assert_eq!(sparq_engine::query(&store.graph, alice_prov).unwrap().rows.len(), 1,
        "alice's bridged grant is marked in provenance");
    let bob_prov = "SELECT ?p ?o WHERE { GRAPH <urn:sparq:auth-bridged> { \
        <https://bob.ex/card#me> ?p ?o } }";
    assert_eq!(sparq_engine::query(&store.graph, bob_prov).unwrap().rows.len(), 0,
        "bob's STATIC grant is NOT in provenance");
}

// 26. A STATIC RE-MATERIALIZATION re-applies still-valid bridged grants (reconcile):
//     materialize_wac rebuilds <urn:sparq:auth> wholesale, but a valid bridged grant is
//     replayed back on top — and a static grant change still takes effect.
#[test]
fn static_rematerialization_preserves_valid_bridged_grant() {
    let nq = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n1.acl> .
"#;
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
    store.materialize_wac().expect("wac");
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission(&read_policy(), &req).granted);

    fn reads_n1(s: &mut PodStore, agent: &str) -> bool {
        let sess = Session { agent: Some(agent), client: None, issuer: None, now: None };
        s.accessible(&sess, Mode::Read).iter().any(|g| g.as_str() == N1)
    }
    assert!(reads_n1(&mut store, ALICE), "alice bridged before re-materialize");

    // A wholesale static re-materialization would normally CLOBBER the bridged grant.
    store.materialize_wac().expect("re-materialize");
    assert!(reads_n1(&mut store, BOB), "bob static after re-materialize");
    assert!(
        reads_n1(&mut store, ALICE),
        "RECONCILE: the valid bridged grant survives a wholesale static re-materialization",
    );
}

// 27. FAIL-CLOSED on AMBIGUOUS re-eval: a windowed grant refreshed with NO dateTime
//     evidence (cannot prove the window still holds) is RETRACTED (never left stale).
#[test]
fn ambiguous_reeval_retracts_fail_closed() {
    let mut store = PodStore::new(pod());
    let pol = windowed_read_policy();
    let in_window = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .with(odrl("dateTime"), Value::DateTime("2025-06-01T00:00:00Z".to_owned()));
    assert!(store.materialize_odrl_permission(&pol, &in_window).granted);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Read).iter().any(|g| g.as_str() == N1));

    // Refresh with NO dateTime evidence → constraint cannot be proven → fail-closed Deny.
    let no_evidence = Request::new(odrl("read")).on(N1).by(ALICE);
    let (matched, retracted) =
        store.refresh_odrl_grant(&pol, &no_evidence, BridgeKind::Permission);
    assert!(matched);
    assert_eq!(retracted, 1, "ambiguous re-eval is retracted, not left stale");
    assert!(
        store.accessible(&alice, Mode::Read).is_empty(),
        "FAIL-CLOSED: no evidence the window holds → access retracted",
    );
}

// ===========================================================================
// [OPUS-4.8] sq-2pcf — DENY RETRACTION on prohibition withdrawal. The symmetric
// dual of sq-dpk4's grant retraction, but with the OPPOSITE fail-closed bias: a
// materialized `auth:deny*` is retracted (access restored) ONLY when the underlying
// ODRL Prohibition is DEFINITELY withdrawn / lapsed — on an *ambiguous* re-eval the
// deny is KEPT (never restore access on missing evidence). These tests drive the
// real enforcement path (`accessible` / `query_as` / `update_as`).
// ===========================================================================

/// alice is prohibited from writing n1 ONLY while a window holds (dateTime < bound).
/// Used to exercise definite-lapse vs ambiguous (no-evidence) deny retraction.
fn windowed_write_prohibition() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/winprohib> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:modify ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
                      odrl:rightOperand "2026-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

/// A policy that prohibits NOTHING (the prohibition has been WITHDRAWN entirely).
fn empty_prohibition_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> . <urn:pol/prohib> a odrl:Set ."#,
        "turtle",
    )
    .expect("policy parses")
}

// 28. WITHDRAWN PROHIBITION → DENY RETRACTED → access RESTORED through the real
//     enforcement path — but ONLY because a still-valid allow grant re-exposes the
//     slot. A standalone deny that is withdrawn restores nothing (test 29).
#[test]
fn withdrawn_prohibition_restores_access_after_refresh() {
    let mut store = PodStore::new(pod());
    let wreq = Request::new(odrl("modify")).on(N1).by(ALICE);

    // alice has a (still-valid, unconstrained) WRITE permit AND a matching prohibition
    // → deny-overrides denies write now.
    let permit = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/w> a odrl:Set ; odrl:permission [
    odrl:action odrl:modify ; odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();
    assert!(store.materialize_odrl_permission(&permit, &wreq).granted);
    assert!(store.materialize_odrl_prohibition(&write_prohibition(), &wreq).prohibited);

    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(
        store.accessible(&alice, Mode::Write).is_empty(),
        "deny-overrides: alice is denied write while the prohibition holds",
    );

    // The Prohibition is WITHDRAWN entirely → refresh the deny entry against the empty
    // policy. The deny is DEFINITELY gone (no prohibition structurally names the request).
    let (matched, retracted) =
        store.refresh_odrl_grant(&empty_prohibition_policy(), &wreq, BridgeKind::Prohibition);
    assert!(matched, "the tracked deny slot matched");
    assert_eq!(retracted, 1, "the withdrawn prohibition's deny was retracted");

    // ACCESS RESTORED: deny gone + the allow grant survives → alice can write again.
    assert!(
        store.accessible(&alice, Mode::Write).iter().any(|g| g.as_str() == N1),
        "deny retracted + allow grant intact → write access restored",
    );
    // And the write-path update enforcement now permits it.
    let ins = "INSERT DATA { GRAPH <https://pod.ex/notes/n1> { \
        <https://pod.ex/notes/n1#it> <https://ex.dev/ns#tag> \"y\" } }";
    assert!(store.update_as(&alice, ins).is_ok(), "restored write update succeeds");
    // No residual deny triple in the auth view.
    let leftover = "SELECT ?o WHERE { GRAPH <urn:sparq:auth> { \
        <https://alice.ex/card#me> <https://sparq.dev/ns/auth#denyWrite> ?o } }";
    assert_eq!(sparq_engine::query(&store.graph, leftover).unwrap().rows.len(), 0,
        "no residual denyWrite after retraction");
}

// 29. WITHDRAWN STANDALONE DENY restores NOTHING: a deny with no underlying allow grant,
//     when retracted, does not widen access (the lack of a grant still denies — the deny
//     retraction is fail-closed by construction).
#[test]
fn withdrawn_standalone_deny_grants_no_access() {
    let mut store = PodStore::new(pod());
    let wreq = Request::new(odrl("modify")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_prohibition(&write_prohibition(), &wreq).prohibited);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Write).is_empty(), "no grant → denied");

    // Withdraw the prohibition; the deny is retracted but there was never an allow.
    let (matched, retracted) =
        store.refresh_odrl_grant(&empty_prohibition_policy(), &wreq, BridgeKind::Prohibition);
    assert!(matched);
    assert_eq!(retracted, 1, "the standalone deny is retracted");
    assert!(
        store.accessible(&alice, Mode::Write).is_empty(),
        "retracting a standalone deny restores no access (no grant to re-expose)",
    );
}

// 30. A STILL-APPLICABLE prohibition SURVIVES refresh — the deny is KEPT, access stays
//     denied (no spurious restoration).
#[test]
fn applicable_prohibition_survives_refresh() {
    let mut store = PodStore::new(pod());
    let wreq = Request::new(odrl("modify")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_prohibition(&write_prohibition(), &wreq).prohibited);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Write).is_empty());

    // Plain refresh (policy unchanged) → the prohibition still matches → deny KEPT.
    assert_eq!(store.refresh_odrl_grants(), 0, "applicable deny not retracted");
    assert!(
        store.accessible(&alice, Mode::Write).is_empty(),
        "still-applicable prohibition: deny survives refresh, access stays denied",
    );
    // Refresh against the SAME prohibition policy also keeps it.
    let (matched, retracted) =
        store.refresh_odrl_grant(&write_prohibition(), &wreq, BridgeKind::Prohibition);
    assert!(matched);
    assert_eq!(retracted, 0, "deny kept on an unchanged, still-matching prohibition");
    assert!(store.accessible(&alice, Mode::Write).is_empty(), "stays denied");
}

// 31. CORE sq-2pcf: AMBIGUOUS re-eval of a windowed prohibition KEEPS the deny
//     (fail-closed) — access is NOT restored when we cannot prove the carve-out is gone.
//     This is the asymmetry vs grant retraction (a windowed GRANT is dropped on ambiguity).
#[test]
fn ambiguous_prohibition_reeval_keeps_deny_fail_closed() {
    let mut store = PodStore::new(pod());
    // A windowed prohibition that holds at materialization time (now < bound).
    let in_window = Request::new(odrl("modify"))
        .on(N1)
        .by(ALICE)
        .with(odrl("dateTime"), Value::DateTime("2025-06-01T00:00:00Z".to_owned()));
    assert!(store
        .materialize_odrl_prohibition(&windowed_write_prohibition(), &in_window)
        .prohibited);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Write).is_empty(), "denied while window holds");

    // Refresh with NO dateTime evidence → we CANNOT prove the window lapsed → AMBIGUOUS.
    // The deny must be KEPT (fail-closed: do NOT restore access on missing evidence).
    let no_evidence = Request::new(odrl("modify")).on(N1).by(ALICE);
    let (matched, retracted) = store.refresh_odrl_grant(
        &windowed_write_prohibition(),
        &no_evidence,
        BridgeKind::Prohibition,
    );
    assert!(matched);
    assert_eq!(retracted, 0, "AMBIGUOUS deny is KEPT, not retracted");
    assert!(
        store.accessible(&alice, Mode::Write).is_empty(),
        "FAIL-CLOSED: no evidence the prohibition lapsed → deny kept → access stays denied",
    );
    // The denyWrite triple is still present in the auth view (re-emitted on refresh).
    let still = "SELECT ?o WHERE { GRAPH <urn:sparq:auth> { \
        <https://alice.ex/card#me> <https://sparq.dev/ns/auth#denyWrite> ?o } }";
    assert_eq!(sparq_engine::query(&store.graph, still).unwrap().rows.len(), 1,
        "ambiguous deny re-emitted (kept) in the enforcement view");
}

// 32. DEFINITELY-LAPSED window → deny RETRACTED: when the refresh request supplies
//     evidence the window has PROVABLY lapsed (now >= bound), the carve-out is known
//     gone → deny retracted. Paired with an allow grant → access restored.
#[test]
fn definitely_lapsed_prohibition_retracts_deny() {
    let mut store = PodStore::new(pod());

    // alice has a still-valid WRITE permit + a windowed prohibition (holds now).
    let permit = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/w> a odrl:Set ; odrl:permission [
    odrl:action odrl:modify ; odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();
    let permit_req = Request::new(odrl("modify")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission(&permit, &permit_req).granted);
    let in_window = Request::new(odrl("modify"))
        .on(N1)
        .by(ALICE)
        .with(odrl("dateTime"), Value::DateTime("2025-06-01T00:00:00Z".to_owned()));
    assert!(store
        .materialize_odrl_prohibition(&windowed_write_prohibition(), &in_window)
        .prohibited);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Write).is_empty(), "denied while window holds");

    // Refresh with evidence the window has PROVABLY lapsed (now >= 2026-01-01 bound,
    // operator is `lt`, so now is NOT < bound → constraint definitely false → Withdrawn).
    let now_lapsed = Request::new(odrl("modify"))
        .on(N1)
        .by(ALICE)
        .with(odrl("dateTime"), Value::DateTime("2026-06-16T00:00:00Z".to_owned()));
    let (matched, retracted) = store.refresh_odrl_grant(
        &windowed_write_prohibition(),
        &now_lapsed,
        BridgeKind::Prohibition,
    );
    assert!(matched);
    assert_eq!(retracted, 1, "provably-lapsed prohibition's deny is retracted");
    assert!(
        store.accessible(&alice, Mode::Write).iter().any(|g| g.as_str() == N1),
        "provably-lapsed deny retracted + allow intact → write access restored",
    );
}

// 33. A STATIC WAC grant is NEVER retracted by a bridged-deny refresh — only the bridged
//     deny is. bob's static read grant survives the full ledger refresh.
#[test]
fn static_grant_never_dropped_by_deny_refresh() {
    let nq = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n1.acl> .
"#;
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
    store.materialize_wac().expect("wac materializes");

    fn reads_n1(s: &mut PodStore, agent: &str) -> bool {
        let sess = Session { agent: Some(agent), client: None, issuer: None, now: None };
        s.accessible(&sess, Mode::Read).iter().any(|g| g.as_str() == N1)
    }
    assert!(reads_n1(&mut store, BOB), "bob static read before");

    // Bridge alice's WRITE prohibition (a bridged deny) on top of the static baseline.
    let wreq = Request::new(odrl("modify")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_prohibition(&write_prohibition(), &wreq).prohibited);

    // Withdraw alice's prohibition → refresh. The bridged deny is retracted; bob's STATIC
    // grant (in the captured baseline, never in the ledger) is untouched.
    let (matched, retracted) =
        store.refresh_odrl_grant(&empty_prohibition_policy(), &wreq, BridgeKind::Prohibition);
    assert!(matched);
    assert_eq!(retracted, 1, "only the bridged deny is retracted");
    assert!(
        reads_n1(&mut store, BOB),
        "STATIC GRANT PRESERVED across a bridged-deny refresh",
    );
}

// 34. DENY-OVERRIDES composition stays correct ACROSS a deny refresh: a permit + a
//     prohibition bridged via a single Policy entry; the refresh re-applies the allow and
//     re-evaluates the deny with the fail-closed deny rule. A withdrawn prohibition (in
//     the refreshed Policy) drops the deny and re-exposes the allow.
#[test]
fn policy_refresh_deny_overrides_composition() {
    let mut store = PodStore::new(pod());
    let both = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/both> a odrl:Set ;
    odrl:permission [ odrl:action odrl:modify ; odrl:target <https://pod.ex/notes/n1> ;
        odrl:assignee <https://alice.ex/card#me> ] ;
    odrl:prohibition [ odrl:action odrl:modify ; odrl:target <https://pod.ex/notes/n1> ;
        odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();
    let req = Request::new(odrl("modify")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_policy(&both, &req).prohibited);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Write).is_empty(), "deny-overrides denies");

    // Refresh against a Policy that keeps the permission but DROPS the prohibition.
    let permit_only = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/both> a odrl:Set ;
    odrl:permission [ odrl:action odrl:modify ; odrl:target <https://pod.ex/notes/n1> ;
        odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();
    let (matched, _retracted) = store.refresh_odrl_grant(&permit_only, &req, BridgeKind::Policy);
    assert!(matched);
    assert!(
        store.accessible(&alice, Mode::Write).iter().any(|g| g.as_str() == N1),
        "deny dropped + allow re-applied → write restored under deny-overrides",
    );
}

// ===========================================================================
// [OPUS-4.8] sq-q56r — faithful odrl:purpose enforcement THROUGH THE REAL
// enforcement path (PodStore::accessible / query_as). A purpose-gated permission
// grants ONLY when the request states a matching purpose; a mismatch denies; a
// MISSING purpose fails closed (no grant); the prohibition dual carves out only on
// a matching stated purpose, and a missing purpose does NOT withdraw the carve-out.
// Match is exact (no hierarchy). All assertions go through accessible / query_as.
// ===========================================================================

const RESEARCH: &str = "urn:purpose/research";
const MARKETING: &str = "urn:purpose/marketing";

/// alice MAY read n1, gated on purpose = research (exact IRI).
fn purpose_read_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/purp> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
                      odrl:rightOperand <urn:purpose/research> ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

fn reads_n1(store: &mut PodStore, agent: &str) -> bool {
    let s = Session { agent: Some(agent), client: None, issuer: None, now: None };
    store.accessible(&s, Mode::Read).iter().any(|g| g.as_str() == N1)
}

// 28. purpose MATCH grants through the real enforcement path; mismatch + missing deny.
#[test]
fn purpose_match_grants_through_enforcement() {
    let pol = purpose_read_policy();
    // [OPUS-4.8] sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };

    // (a) Matching purpose → grant → alice reads through accessible AND query_as.
    let mut store = PodStore::new(pod());
    let ok = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .for_purpose(Value::Iri(RESEARCH.to_owned()));
    assert!(store.materialize_odrl_permission(&pol, &ok).granted, "matching purpose grants");
    assert!(reads_n1(&mut store, ALICE), "alice reads with matching purpose");
    assert_eq!(store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 1);

    // (b) Mismatched purpose → no grant, nothing readable.
    let mut store2 = PodStore::new(pod());
    let bad = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .for_purpose(Value::Iri(MARKETING.to_owned()));
    assert!(!store2.materialize_odrl_permission(&pol, &bad).granted, "mismatch denies");
    assert!(!reads_n1(&mut store2, ALICE), "no access on purpose mismatch");
    assert_eq!(store2.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 0);
}

// 29. THE honesty test: a MISSING purpose fails closed — no grant, no access. "No
//     purpose stated" is never silently treated as "any purpose allowed".
#[test]
fn missing_purpose_fails_closed_through_enforcement() {
    let mut store = PodStore::new(pod());
    let no_purpose = Request::new(odrl("read")).on(N1).by(ALICE); // no purpose evidence
    let out = store.materialize_odrl_permission(&purpose_read_policy(), &no_purpose);
    assert!(!out.granted, "missing purpose must NOT grant: {out:?}");
    assert!(!reads_n1(&mut store, ALICE), "no access when purpose is unstated");
    // [OPUS-4.8] sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert_eq!(store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 0);
}

// 30. Match is EXACT — a narrower sub-purpose IRI is not subsumed (no hierarchy).
#[test]
fn purpose_match_is_exact_through_enforcement() {
    let mut store = PodStore::new(pod());
    let sub = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .for_purpose(Value::Iri("urn:purpose/research/clinical".to_owned()));
    assert!(
        !store.materialize_odrl_permission(&purpose_read_policy(), &sub).granted,
        "exact-match only: a sub-purpose IRI is not subsumed",
    );
    assert!(!reads_n1(&mut store, ALICE));
}

// 31. The DUAL — a purpose-gated PROHIBITION carves out (denies) only on a matching
//     stated purpose; a different purpose does NOT carve out; a MISSING purpose does
//     NOT withdraw the carve-out (deny stays — fail-closed). All via accessible.
#[test]
fn purpose_prohibition_dual_through_enforcement() {
    // A standalone permit (any purpose) so the prohibition has an allow to override.
    let permit = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();
    // alice is PROHIBITED from reading n1 FOR purpose = marketing.
    let prohib = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/pp> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
                      odrl:rightOperand <urn:purpose/marketing> ] ] .
"#,
        "turtle",
    )
    .unwrap();

    // (a) Stated marketing purpose → prohibition carves out → DENY beats the allow.
    let mut store = PodStore::new(pod());
    let unconstrained = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission(&permit, &unconstrained).granted);
    assert!(reads_n1(&mut store, ALICE), "allow live before the deny");
    let marketing = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .for_purpose(Value::Iri(MARKETING.to_owned()));
    let out = store.materialize_odrl_prohibition(&prohib, &marketing);
    assert!(out.prohibited, "matching purpose carves out: {out:?}");
    assert!(!reads_n1(&mut store, ALICE), "deny-overrides: marketing purpose denied");

    // (b) Stated a DIFFERENT purpose → prohibition does NOT carve out (no deny).
    let mut store2 = PodStore::new(pod());
    assert!(store2.materialize_odrl_permission(&permit, &unconstrained).granted);
    let research = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .for_purpose(Value::Iri(RESEARCH.to_owned()));
    let out2 = store2.materialize_odrl_prohibition(&prohib, &research);
    assert!(!out2.prohibited, "a non-marketing purpose is not carved out: {out2:?}");
    assert!(reads_n1(&mut store2, ALICE), "allow survives: research purpose not prohibited");

    // (c) NO purpose stated → the carve-out is *unprovable*, so materialize_prohibition
    //     emits NO deny: it materializes a deny only when the prohibition DEFINITELY
    //     matches (the matched_prohibition boundary). The deny is materialized once a
    //     request actually states the banned purpose. The *deny-retraction* direction —
    //     where unprovable must NOT restore an existing deny — is prohibition_status's
    //     job (ProhibitionStatus::Ambiguous keeps it; asserted in sparq-policy's tests).
    let mut store3 = PodStore::new(pod());
    assert!(store3.materialize_odrl_permission(&permit, &unconstrained).granted);
    let out3 = store3.materialize_odrl_prohibition(&prohib, &unconstrained);
    assert!(!out3.prohibited, "no purpose evidence → no definite carve-out materialized");
}

// ===========================================================================
// [OPUS-4.8] sq-5037 — `odrl:recipient neq X` / "everyone-except-X" → an ACP
// `noneOf` exception: the grant head is auth:Public with an auth:exceptMatcher
// carving out X. RE-CHECKED per session: everyone reads EXCEPT X (and anonymous,
// who fails the public head? no — public matches anonymous; X is excluded).
// ===========================================================================

const DAVE: &str = "https://dave.ex/card#me";

/// "everyone EXCEPT bob may read n1" — recipient neq bob.
fn recipient_neq_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/neq> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ;
                      odrl:operator odrl:neq ;
                      odrl:rightOperand <https://bob.ex/card#me> ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

/// The `auth:exceptMatcher` IRIs carved out by ConditionalGrants in `graph`, paired
/// with the agent each matcher accepts (the noneOf shape the session layer reads).
fn except_matchers(graph: &Graph) -> Vec<(String, String)> {
    let q = "SELECT ?m ?a WHERE { GRAPH <urn:sparq:auth> { \
        ?g <https://sparq.dev/ns/auth#exceptMatcher> ?m . \
        ?m <https://sparq.dev/ns/solidx#acceptsAgentP> ?a } }";
    sparq_engine::query(graph, q)
        .expect("query")
        .rows
        .iter()
        .map(|r| (format!("{:?}", r[0]), format!("{:?}", r[1])))
        .collect()
}

// 18. The noneOf BRIDGE SHAPE: a recipient-neq permission materializes a public
//     ConditionalGrant with an exceptMatcher accepting (agent = bob, client = any).
#[test]
fn recipient_neq_emits_noneof_exception_shape() {
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &recipient_neq_policy(), &req);
    assert!(out.granted, "recipient-neq maps to a noneOf condition: {out:?}");

    // Exactly one public ConditionalGrant (the "everyone" head) — NOT one per agent.
    assert_eq!(
        cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        1,
        "everyone-except is a single public head"
    );
    // ... carrying an exception matcher that accepts bob (the carved-out party).
    let ms = except_matchers(&g);
    assert_eq!(ms.len(), 1, "one exceptMatcher: {ms:?}");
    assert!(ms[0].1.contains("bob.ex"), "exception carves out bob: {ms:?}");
}

// 19. RE-CHECKED end-to-end through the enforcement path: everyone reads EXCEPT bob.
//     Anonymous (public) reads; bob is denied; the materializing party (alice) reads.
#[test]
fn recipient_neq_grants_everyone_except_named_party() {
    let pol = recipient_neq_policy();
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);

    assert!(reads(&mut store, CAROL), "carol (not bob) granted");
    assert!(reads(&mut store, DAVE), "dave (not bob) granted");
    assert!(reads(&mut store, ALICE), "alice (not bob) granted");
    assert!(!reads(&mut store, BOB), "bob is carved out by the noneOf exception");
    // The public head matches anonymous too (no party named ⇒ everyone-except).
    assert!(
        store.accessible(&Session::default(), Mode::Read).iter().any(|gr| gr.as_str() == N1),
        "anonymous (public) is granted; only bob is excepted"
    );

    // End-to-end via query_as: bob sees nothing, carol sees the content.
    // [OPUS-4.8] sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let bob = Session { agent: Some(BOB), client: None, issuer: None, now: None };
    let carol = Session { agent: Some(CAROL), client: None, issuer: None, now: None };
    assert_eq!(store.query_as(&bob, Mode::Read, sel).unwrap().rows.len(), 0);
    assert_eq!(store.query_as(&carol, Mode::Read, sel).unwrap().rows.len(), 1);
}

// 20. The PROHIBITION dual: a prohibition `recipient neq bob` carves out everyone
//     EXCEPT bob (deny-overrides). Per the bridge's deny-overrides check, ANY matching
//     prohibition blocks the grant; here the conditional path emits nothing because the
//     prohibition matches the request party (alice != bob → neq holds → carved out).
#[test]
fn recipient_neq_prohibition_blocks_grant() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/pneq> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ;
      odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
                        odrl:rightOperand <https://bob.ex/card#me> ] ] .
"#,
        "turtle",
    )
    .unwrap();
    // alice (≠ bob) → the prohibition's neq holds → deny-overrides → nothing granted.
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(!out.granted, "prohibition recipient-neq carves out non-bob: {out:?}");
    assert_eq!(cond_grants_for(&g, None), 0, "deny-overrides → no conditional grant");
}

// 21. A reserved-encoded neq recipient cannot become an enforceable per-session matcher
//     (it could otherwise impersonate a minted pair principal), so the bridge must NOT
//     emit an "everyone-except" public noneOf grant for it — that would re-admit the
//     carved-out party. Instead the rule falls back to the one-shot path, which checks
//     the neq (frozen) against the materializing party and grants ONLY that party — no
//     widening to public, and no unguarded public grant leaked.
#[test]
fn recipient_neq_reserved_encoded_does_not_widen_to_public() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/rsv> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
                      odrl:rightOperand <urn:sparq:pair?agent=x&client=y> ] ] .
"#,
        "turtle",
    )
    .unwrap();
    let mut g = pod();
    // alice (≠ the reserved party) → the one-shot path proves the neq for alice and
    // grants alice (frozen). Crucially NO public noneOf grant is emitted — the reserved
    // exclusion cannot become a matcher, so access is NOT widened to everyone-except.
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(out.granted, "one-shot grants the (non-excluded) materializing party: {out:?}");
    assert_eq!(
        cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "reserved-encoded neq must NOT widen to a public everyone-except grant"
    );
    assert!(except_matchers(&g).is_empty(), "no unenforceable matcher emitted");

    // Frozen, scoped to alice: a stranger is denied (no widening to public).
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(reads(&mut store, ALICE), "alice (the materializer, non-excluded) granted");
    assert!(!reads(&mut store, CAROL), "no public widening from a reserved exclusion");
}

// ===========================================================================
// [OPUS-4.8] sq-gx2q — REFRESH of a noneOf conditional grant when the EXCLUSION SET
// CHANGES (sq-5037 follow-up, sq-dpk4 interaction). A `recipient neq X` permission
// bridges to a public ConditionalGrant carrying a `noneOf` exceptMatcher that carves
// out X (everyone-except-X). When the ODRL policy later swaps the excluded party (X→Y),
// `refresh_odrl_grant(BridgeKind::PermissionConditional)` must RETRACT the X carve-out
// (X regains access) and REPLAY the Y carve-out (Y loses access) — through the real
// per-session enforcement path — leaving NO residual old matcher. This is the union
// of sq-5037's noneOf shape and sq-dpk4's refresh-as-baseline-reset-then-replay.
// ===========================================================================

/// "everyone EXCEPT `<excluded>` may read n1" — a `recipient neq <excluded>` permission
/// (the noneOf "everyone-except-X" shape). Parameterised so the exclusion set can be
/// swapped between refreshes. [OPUS-4.8] sq-gx2q.
fn recipient_neq_policy_excluding(excluded: &str) -> sparq_policy::Policy {
    parse_policy_str(
        &format!(
            r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/neq> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ;
                      odrl:operator odrl:neq ;
                      odrl:rightOperand <{excluded}> ] ] .
"#
        ),
        "turtle",
    )
    .expect("policy parses")
}

// 22. CHANGED EXCLUSION SET: an everyone-except-BOB noneOf grant is refreshed against an
//     everyone-except-DAVE policy → the bob carve-out is retracted (bob regains access),
//     the dave carve-out is replayed (dave loses access), and the OLD bob exceptMatcher
//     leaves no residue. Verified through `accessible` AND the raw exceptMatcher shape.
#[test]
fn refresh_noneof_grant_replays_changed_exclusion_set() {
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);

    // Bridge "everyone EXCEPT bob" → bob is the sole carved-out party.
    assert!(store
        .materialize_odrl_permission_conditional(&recipient_neq_policy_excluding(BOB), &req)
        .granted);
    assert!(!reads(&mut store, BOB), "bob excluded before refresh");
    assert!(reads(&mut store, DAVE), "dave granted before refresh");
    assert!(reads(&mut store, CAROL), "carol granted before refresh");
    {
        let ms = except_matchers(&store.graph);
        assert_eq!(ms.len(), 1, "exactly one exceptMatcher before refresh: {ms:?}");
        assert!(ms[0].1.contains("bob.ex"), "carve-out names bob: {ms:?}");
    }

    // The policy SWAPS the excluded party: now "everyone EXCEPT dave". refresh against the
    // changed exclusion set re-evaluates the same tracked slot (kind/target/party match —
    // the request party is the materializer, not the excluded recipient).
    let (matched, retracted) = store.refresh_odrl_grant(
        &recipient_neq_policy_excluding(DAVE),
        &req,
        BridgeKind::PermissionConditional,
    );
    assert!(matched, "the tracked conditional-grant slot matched");
    // The entry still materializes a grant (everyone-except-dave), so it is NOT counted as
    // retracted — but its exclusion CARVE-OUT was replayed, swapping bob for dave.
    assert_eq!(retracted, 0, "the grant still holds (re-headed), so not retracted");

    // ENFORCEMENT FLIP: bob regains access (the bob carve-out is gone); dave loses it.
    assert!(reads(&mut store, BOB), "RETRACTED carve-out: bob regains read access");
    assert!(!reads(&mut store, DAVE), "REPLAYED carve-out: dave is now excluded");
    assert!(reads(&mut store, CAROL), "carol (never excluded) keeps access");
    assert!(
        store.accessible(&Session::default(), Mode::Read).iter().any(|gr| gr.as_str() == N1),
        "anonymous (public) still granted; only dave is excepted now"
    );

    // The auth view holds exactly ONE exceptMatcher, and it carves out DAVE — the stale
    // bob matcher left no residue (baseline reset + provenance clear before replay).
    let ms = except_matchers(&store.graph);
    assert_eq!(ms.len(), 1, "exactly one exceptMatcher after refresh (no stale bob): {ms:?}");
    assert!(ms[0].1.contains("dave.ex"), "carve-out now names dave: {ms:?}");
    assert!(!ms[0].1.contains("bob.ex"), "no residual bob carve-out: {ms:?}");

    // End-to-end through query_as confirms the flip at the query layer.
    // [OPUS-4.8] sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let bob = Session { agent: Some(BOB), client: None, issuer: None, now: None };
    let dave = Session { agent: Some(DAVE), client: None, issuer: None, now: None };
    assert_eq!(store.query_as(&bob, Mode::Read, sel).unwrap().rows.len(), 1, "bob now reads");
    assert_eq!(store.query_as(&dave, Mode::Read, sel).unwrap().rows.len(), 0, "dave now denied");
}

// 23. EXCLUSION SET GROWS then the WHOLE permission is WITHDRAWN: refreshing a noneOf
//     grant against an empty policy retracts it entirely — the public head AND every
//     carve-out are gone, and access is fully removed (fail-closed). This is the noneOf
//     analogue of `withdrawn_permission_loses_access_after_refresh` (sq-dpk4).
#[test]
fn refresh_noneof_grant_withdrawn_retracts_public_head() {
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);

    // Bridge "everyone EXCEPT bob" → carol/dave/anonymous read, bob does not.
    assert!(store
        .materialize_odrl_permission_conditional(&recipient_neq_policy_excluding(BOB), &req)
        .granted);
    assert!(reads(&mut store, CAROL), "carol reads before withdrawal");
    assert!(
        store.accessible(&Session::default(), Mode::Read).iter().any(|gr| gr.as_str() == N1),
        "anonymous reads before withdrawal"
    );

    // The permission is WITHDRAWN entirely → refresh against the empty policy.
    let (matched, retracted) =
        store.refresh_odrl_grant(&empty_policy(), &req, BridgeKind::PermissionConditional);
    assert!(matched, "the tracked conditional-grant slot matched");
    assert_eq!(retracted, 1, "the whole everyone-except grant was retracted");

    // FAIL-CLOSED: the public head AND the carve-out are gone — nobody reads via the bridge.
    assert!(!reads(&mut store, CAROL), "STALE noneOf GRANT MUST LOSE ACCESS: carol denied");
    assert!(!reads(&mut store, DAVE), "dave denied after withdrawal");
    assert!(
        store.accessible(&Session::default(), Mode::Read).is_empty(),
        "anonymous (public) denied after withdrawal"
    );
    assert!(except_matchers(&store.graph).is_empty(), "no residual exceptMatcher");
    assert_eq!(
        cond_grants_for(&store.graph, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "no residual public head after retraction"
    );
}

// ===========================================================================
// [OPUS-4.8] sq-5037 follow-up — COMBINED recipient `eq A AND neq B` in ONE rule
// through the BRIDGE. The bridge emits `agents=[A]` (positive head) + `except=[B]`
// (a noneOf exceptMatcher) — both on the SAME ConditionalGrant. Structurally emitted
// by sq-5037 but untested at the bridge level (the per-head exception path).
// ===========================================================================

/// "carol (and only carol), but never bob, may read n1" — recipient eq carol AND neq bob.
fn recipient_eq_and_neq_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/comb> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
                      odrl:rightOperand <https://carol.ex/card#me> ] ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
                      odrl:rightOperand <https://bob.ex/card#me> ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

// 22. The COMBINED bridge shape: a single ConditionalGrant whose positive head is carol
//     AND which carries an exceptMatcher carving out bob (the per-head exception).
#[test]
fn recipient_eq_and_neq_emits_carol_head_with_bob_exception() {
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &recipient_eq_and_neq_policy(), &req);
    assert!(out.granted, "combined eq+neq maps to a faithful condition: {out:?}");

    // Exactly one ConditionalGrant, headed by carol (the positive eq constraint) — NOT
    // a public head (the eq narrows it to carol).
    assert_eq!(cond_grants_for(&g, Some(CAROL)), 1, "carol positive head present");
    assert_eq!(
        cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "eq carol means the head is carol, not public"
    );
    // ... carrying an exception matcher that carves out bob (the neq constraint).
    let ms = except_matchers(&g);
    assert_eq!(ms.len(), 1, "one exceptMatcher (the neq bob carve-out): {ms:?}");
    assert!(ms[0].1.contains("bob.ex"), "exception carves out bob: {ms:?}");
}

// 23. RE-CHECKED end-to-end: only carol reads. Bob is excluded by BOTH the eq head (he
//     is not carol) AND the neq exception; everyone else is excluded by the eq head.
#[test]
fn recipient_eq_and_neq_grants_only_carol() {
    let pol = recipient_eq_and_neq_policy();
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);

    assert!(reads(&mut store, CAROL), "carol satisfies the eq head and is not excepted");
    assert!(!reads(&mut store, BOB), "bob is not carol AND is excepted");
    assert!(!reads(&mut store, DAVE), "dave is not carol → eq head excludes him");
    assert!(!reads(&mut store, ALICE), "the materializer is not auto-granted");
    assert!(store.accessible(&Session::default(), Mode::Read).is_empty(), "anonymous denied");

    // End-to-end via query_as: only carol sees the content.
    // [OPUS-4.8] sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let carol = Session { agent: Some(CAROL), client: None, issuer: None, now: None };
    let bob = Session { agent: Some(BOB), client: None, issuer: None, now: None };
    assert_eq!(store.query_as(&carol, Mode::Read, sel).unwrap().rows.len(), 1);
    assert_eq!(store.query_as(&bob, Mode::Read, sel).unwrap().rows.len(), 0);
}

// ===========================================================================
// [OPUS-4.8] sq-4r70 — CONSTRAINT-CONDITIONAL DENY: an ACP-style deny that is
// conditional on a recipient/assignee constraint (the dual of the conditional grant).
// Materialized as a re-checked `auth:ConditionalGrant` with `auth:effect auth:Deny`,
// composing with deny-overrides. The deny appears/retracts as the condition flips.
// ===========================================================================
use sparq_solid::materialize_prohibition_conditional;

/// Count `auth:ConditionalGrant` heads with `auth:effect auth:Deny` naming `agent`
/// (or any) in `graph`'s auth view (the conditional-DENY dual of `cond_grants_for`).
fn cond_denies_for(graph: &Graph, agent: Option<&str>) -> usize {
    let head = "?g <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
        <https://sparq.dev/ns/auth#ConditionalGrant> ; \
        <https://sparq.dev/ns/auth#effect> <https://sparq.dev/ns/auth#Deny>";
    let q = match agent {
        Some(a) => format!(
            "SELECT ?g WHERE {{ GRAPH <urn:sparq:auth> {{ {head} ; \
             <https://sparq.dev/ns/auth#agent> <{a}> }} }}"
        ),
        None => format!("SELECT ?g WHERE {{ GRAPH <urn:sparq:auth> {{ {head} }} }}"),
    };
    sparq_engine::query(graph, &q).expect("query").rows.len()
}

/// "carol (recipient) is PROHIBITED from reading n1" — a recipient-eq prohibition that
/// the conditional path maps to a per-session deny carving out exactly carol.
fn prohibit_recipient_carol_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/pcond> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
                      odrl:rightOperand <https://carol.ex/card#me> ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

// 24. The conditional-deny BRIDGE SHAPE: a recipient-eq prohibition materializes a
//     ConditionalGrant with auth:effect auth:Deny headed by carol (re-checked, NOT frozen).
#[test]
fn conditional_deny_emits_deny_effect_condition() {
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_prohibition_conditional(&mut g, &prohibit_recipient_carol_policy(), &req);
    assert!(out.prohibited, "faithful recipient prohibition maps to a deny condition: {out:?}");
    assert_eq!(cond_denies_for(&g, Some(CAROL)), 1, "carol deny condition present");
    assert_eq!(cond_denies_for(&g, Some(ALICE)), 0, "no deny for the materializer");
    // It is a DENY, not an allow (the audit anchor reports the effect predicate).
    assert_eq!(out.deny_triple.as_ref().map(|t| t.1.as_str()),
        Some("https://sparq.dev/ns/auth#effect"), "deny anchor: {out:?}");
}

// 25. RE-CHECKED end-to-end with DENY-OVERRIDES: a public allow grant is in force, and a
//     conditional deny carves out carol. carol is denied (deny beats allow); everyone
//     else keeps the allow. This composes the conditional deny with an existing allow.
#[test]
fn conditional_deny_overrides_allow_for_carved_party() {
    let mut store = PodStore::new(pod());
    // A public allow: everyone may read n1 (a bare permission, conditional path → Public).
    let permit = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pub> a odrl:Set ; odrl:permission [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] ."#,
        "turtle",
    )
    .unwrap();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission_conditional(&permit, &req).granted);
    assert!(reads(&mut store, CAROL), "everyone reads before the deny");
    assert!(reads(&mut store, BOB), "bob reads before the deny");

    // Now layer the conditional deny carving out carol.
    let out = store.materialize_odrl_prohibition_conditional(&prohibit_recipient_carol_policy(), &req);
    assert!(out.prohibited, "deny condition materialized: {out:?}");
    assert!(!reads(&mut store, CAROL), "DENY-OVERRIDES: carol loses access");
    assert!(reads(&mut store, BOB), "bob keeps the allow (only carol is denied)");
    // End-to-end query_as: carol sees nothing, bob sees the content.
    // [OPUS-4.8] sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let carol = Session { agent: Some(CAROL), client: None, issuer: None, now: None };
    let bob = Session { agent: Some(BOB), client: None, issuer: None, now: None };
    assert_eq!(store.query_as(&carol, Mode::Read, sel).unwrap().rows.len(), 0);
    assert_eq!(store.query_as(&bob, Mode::Read, sel).unwrap().rows.len(), 1);
}

// 26. The deny APPEARS/RETRACTS as the condition flips: a recipient-neq prohibition maps
//     to a deny on everyone EXCEPT bob; when the prohibition is WITHDRAWN, refresh
//     retracts the deny and access is restored (composes with sq-2pcf deny-retraction).
#[test]
fn conditional_deny_retracts_when_prohibition_withdrawn() {
    let mut store = PodStore::new(pod());
    // Baseline public allow so we can observe the deny biting and then lifting.
    let permit = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pub> a odrl:Set ; odrl:permission [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] ."#,
        "turtle",
    )
    .unwrap();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission_conditional(&permit, &req).granted);

    // "everyone EXCEPT bob is prohibited" → a deny condition with a bob exception.
    let prohib = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pneq> a odrl:Set ; odrl:prohibition [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ;
            odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
                              odrl:rightOperand <https://bob.ex/card#me> ] ] ."#,
        "turtle",
    )
    .unwrap();
    assert!(store.materialize_odrl_prohibition_conditional(&prohib, &req).prohibited);
    // The deny carves out everyone except bob: carol denied, bob keeps the allow.
    assert!(!reads(&mut store, CAROL), "carol (not bob) is denied by the conditional deny");
    assert!(reads(&mut store, BOB), "bob is excepted from the deny → keeps the allow");

    // WITHDRAW the prohibition entirely → refresh → the deny is retracted → access back.
    let empty = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> . <urn:pol/x> a odrl:Set ."#,
        "turtle",
    )
    .unwrap();
    let (matched, retracted) =
        store.refresh_odrl_grant(&empty, &req, BridgeKind::ProhibitionConditional);
    assert!(matched, "the tracked deny slot matched");
    assert_eq!(retracted, 1, "the withdrawn deny condition was retracted");
    assert!(reads(&mut store, CAROL), "deny withdrawn → carol regains access");
    assert_eq!(cond_denies_for(&store.graph, None), 0, "no residual deny condition");
}

// 27. MIXED / unmappable constraint falls back to ONE-SHOT: a recipient-eq + dateTime
//     prohibition cannot persist the time bound as a condition (ACP has no clock), so it
//     falls back to the one-shot deny (frozen, materialized iff the prohibition matches).
#[test]
fn conditional_deny_mixed_constraint_falls_back_one_shot() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/mix> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://carol.ex/card#me> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
                      odrl:rightOperand "2027-01-01T00:00:00Z"^^xsd:dateTime ] ] .
"#,
        "turtle",
    )
    .unwrap();
    // carol asking, with a time INSIDE the window → the one-shot deny materializes
    // (frozen `auth:denyRead`), NOT a re-checked deny condition.
    let mut g = pod();
    let req = Request::new(odrl("read"))
        .on(N1)
        .by(CAROL)
        .with(odrl("dateTime"), Value::DateTime("2026-06-16T00:00:00Z".to_owned()));
    let out = materialize_prohibition_conditional(&mut g, &pol, &req);
    assert!(out.prohibited, "one-shot deny materializes inside the window: {out:?}");
    // No re-checked deny CONDITION emitted — the time bound forced the one-shot path.
    assert_eq!(cond_denies_for(&g, None), 0, "unmappable dateTime → no deny condition");
    // The frozen one-shot deny names carol via auth:denyRead.
    assert_eq!(out.deny_triple.as_ref().map(|t| t.1.contains("denyRead")), Some(true), "{out:?}");
}

// ===========================================================================
// [OPUS-4.8] sq-ihqbl — the bridge LOUDLY REFUSES a policy whose declared
// `odrl:conflict` strategy it cannot faithfully honour (fail-closed), rather than
// silently processing it as deny-overrides. The bridge implements exactly one strategy
// (`odrl:prohibit`); `odrl:perm`, `odrl:invalid`-with-conflict, and any unknown strategy
// are rejected outright. NON-VACUOUS: every refusal policy below has a conflicting
// permission+prohibition on the SAME subject, so under the pre-fix lenient behaviour
// `materialize_policy` would have materialized the deny (deny-overrides) — here it
// materializes NOTHING and flags `refused`.
// ===========================================================================

/// A conflicting modify-permission + modify-prohibition on N1 for alice, with the given
/// `odrl:conflict` clause spliced in (empty = leave unset).
fn conflicting_write_policy(conflict_clause: &str) -> sparq_policy::Policy {
    let ttl = format!(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/both> a odrl:Set ;
    {conflict_clause}
    odrl:permission [
        odrl:action odrl:modify ;
        odrl:target <https://pod.ex/notes/n1> ;
        odrl:assignee <https://alice.ex/card#me> ] ;
    odrl:prohibition [
        odrl:action odrl:modify ;
        odrl:target <https://pod.ex/notes/n1> ;
        odrl:assignee <https://alice.ex/card#me> ] .
"#
    );
    parse_policy_str(&ttl, "turtle").expect("policy parses")
}

/// SUPPORTED strategy: an explicit `odrl:conflict odrl:prohibit` (deny-overrides — the
/// one strategy the bridge implements) still materializes the deny exactly as before.
/// Proves the gate does not over-refuse the strategy it does support.
#[test]
fn explicit_prohibit_strategy_still_materializes_deny() {
    let mut g = pod();
    let req = Request::new(odrl("modify")).on(N1).by(ALICE);
    let out = materialize_policy(&mut g, &conflicting_write_policy("odrl:conflict odrl:prohibit ;"), &req);
    assert!(!out.refused, "the supported strategy is not refused: {out:?}");
    assert!(out.prohibited, "deny-overrides still materializes the deny: {out:?}");
    assert!(!out.granted, "the permit is overridden by the prohibition: {out:?}");
    assert!(out.deny_triple.is_some(), "{out:?}");
}

/// UNSUPPORTED strategy `odrl:perm` (permissions override prohibitions) → REFUSED.
/// Non-vacuous: without the fix this policy materializes a deny (deny-overrides); with
/// the fix it materializes NOTHING and says so loudly.
#[test]
fn perm_strategy_is_refused_and_materializes_nothing() {
    let mut g = pod();
    let req = Request::new(odrl("modify")).on(N1).by(ALICE);
    let out = materialize_policy(&mut g, &conflicting_write_policy("odrl:conflict odrl:perm ;"), &req);

    assert!(out.refused, "odrl:perm must be REFUSED, not silently enforced: {out:?}");
    assert!(!out.granted && !out.prohibited, "a refusal materializes neither side: {out:?}");
    assert!(out.grant_triple.is_none() && out.deny_triple.is_none(), "nothing emitted: {out:?}");
    assert!(
        out.reasons.iter().any(|r| r.contains("REFUSED") && r.contains("perm")),
        "the refusal reason is loud and names the strategy: {out:?}",
    );

    // End-to-end: alice gets NO access through the real enforcement (fail-closed) — the
    // refusal never materialized the (would-be) grant, and the deny that the old path
    // would have written is absent because the whole policy was rejected.
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_policy(&conflicting_write_policy("odrl:conflict odrl:perm ;"), &req).refused);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(
        store.accessible(&alice, Mode::Write).is_empty(),
        "a refused policy grants nothing (fail-closed)",
    );
}

/// `odrl:conflict odrl:invalid` WITH a detected conflict → the policy is void as a whole
/// → REFUSED (materializes nothing). Non-vacuous the same way.
#[test]
fn invalid_strategy_with_conflict_is_refused() {
    let mut g = pod();
    let req = Request::new(odrl("modify")).on(N1).by(ALICE);
    let out = materialize_policy(&mut g, &conflicting_write_policy("odrl:conflict odrl:invalid ;"), &req);
    assert!(out.refused, "odrl:invalid + conflict must be REFUSED: {out:?}");
    assert!(!out.granted && !out.prohibited, "{out:?}");
    assert!(out.reasons.iter().any(|r| r.contains("REFUSED") && r.contains("invalid")), "{out:?}");
}

/// An UNKNOWN `odrl:conflict` strategy IRI → REFUSED. Also verifies the single-side
/// entry points (`materialize_permission` / `materialize_prohibition`) refuse too.
#[test]
fn unknown_strategy_is_refused_on_every_entry_point() {
    let pol = conflicting_write_policy("odrl:conflict <urn:custom:mediate> ;");
    let req = Request::new(odrl("modify")).on(N1).by(ALICE);

    let mut g1 = pod();
    let policy_out = materialize_policy(&mut g1, &pol, &req);
    assert!(policy_out.refused, "materialize_policy refuses unknown strategy: {policy_out:?}");
    assert!(
        policy_out.reasons.iter().any(|r| r.contains("urn:custom:mediate")),
        "the refusal names the offending IRI: {policy_out:?}",
    );

    let mut g2 = pod();
    assert!(materialize_permission(&mut g2, &pol, &req).refused, "permission side refuses too");

    let mut g3 = pod();
    let deny_out = materialize_prohibition(&mut g3, &pol, &req);
    assert!(deny_out.refused, "prohibition side refuses too: {deny_out:?}");
    assert!(!deny_out.prohibited, "and materializes no deny under an unimplementable strategy");
}

/// Regression: a policy that declares NO `odrl:conflict` is unaffected — the unset
/// default is the implemented deny-overrides, so the existing deny materializes as before
/// (the bridge's core use case is not refused).
#[test]
fn unset_conflict_is_not_refused() {
    let mut g = pod();
    let req = Request::new(odrl("modify")).on(N1).by(ALICE);
    let out = materialize_policy(&mut g, &conflicting_write_policy(""), &req);
    assert!(!out.refused, "an undeclared conflict strategy defaults to deny-overrides: {out:?}");
    assert!(out.prohibited, "the deny still materializes: {out:?}");
}

// ===========================================================================
// [FABLE-5] sq-5fkpp — faithful `odrl:isAnyOf` / `odrl:isNoneOf` mapping.
// `recipient isAnyOf <set>` → one re-checked agent head per member (exactly the
// `isPartOf` shape — the evaluator matches both operators as the same flat lexical
// set, sq-uaz85); `recipient isNoneOf <set>` → one ACP noneOf exceptMatcher per
// member (the list-valued `neq` dual, sq-5037). Previously both routed through the
// catch-all to Unmappable, freezing the whole rule one-shot.
// ===========================================================================

/// bob OR carol may read n1 — `recipient isAnyOf "bob|carol"`.
fn recipient_isanyof_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/anyof> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ;
                      odrl:operator odrl:isAnyOf ;
                      odrl:rightOperand "https://bob.ex/card#me|https://carol.ex/card#me" ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

/// everyone EXCEPT bob and dave may read n1 — `recipient isNoneOf "bob|dave"`.
fn recipient_isnoneof_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/noneof> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ;
                      odrl:operator odrl:isNoneOf ;
                      odrl:rightOperand "https://bob.ex/card#me|https://dave.ex/card#me" ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

/// A recipient-`isNoneOf`-style permission with an arbitrary operator + right operand
/// spliced in (for the malformed-operand fail-closed cases).
fn recipient_set_policy(operator: &str, right_operand: &str) -> sparq_policy::Policy {
    let ttl = format!(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/setop> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ;
                      odrl:operator odrl:{operator} ;
                      odrl:rightOperand {right_operand} ] ] .
"#
    );
    parse_policy_str(&ttl, "turtle").expect("policy parses")
}

/// Bridge ↔ evaluator PARITY over identified recipients: after bridging (materialized
/// by ALICE — the constraint need not hold against the materializing party), each
/// candidate agent's per-session access equals `sparq_policy::evaluate`'s verdict for
/// a request naming that party. Anonymous sessions are deliberately OUT of the panel:
/// the evaluator is fail-closed on missing identity, while the bridged noneOf shape
/// keeps the everyone-except public head — the accepted sq-5037 semantics (asserted
/// separately below).
fn assert_bridge_evaluator_parity(pol: &sparq_policy::Policy) {
    let mut store = PodStore::new(pod());
    let mat = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission_conditional(pol, &mat).granted);
    for agent in [ALICE, BOB, CAROL, DAVE] {
        let bridged = reads(&mut store, agent);
        let evaluated =
            sparq_policy::evaluate(pol, &Request::new(odrl("read")).on(N1).by(agent)).allow;
        assert_eq!(bridged, evaluated, "bridge/evaluator parity for {agent}");
    }
}

// 30. `isAnyOf` BRIDGE SHAPE: one re-checked agent head per set member (NOT a public
//     head), exactly as `isPartOf`.
#[test]
fn recipient_isanyof_persists_one_condition_per_member() {
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &recipient_isanyof_policy(), &req);
    assert!(out.granted, "isAnyOf maps faithfully to agent conditions: {out:?}");
    assert_eq!(cond_grants_for(&g, Some(BOB)), 1, "bob head present");
    assert_eq!(cond_grants_for(&g, Some(CAROL)), 1, "carol head present");
    assert_eq!(
        cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "a positive set is per-member heads, never a public head"
    );
    assert!(except_matchers(&g).is_empty(), "no exception on a positive set");

    // RE-CHECKED end-to-end: members read, everyone else (incl. the materializer) not.
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&recipient_isanyof_policy(), &req).granted);
    assert!(reads(&mut store, BOB), "bob in set");
    assert!(reads(&mut store, CAROL), "carol in set");
    assert!(!reads(&mut store, ALICE), "alice (the materializer) not in set");
    assert!(!reads(&mut store, DAVE), "dave not in set");
    assert!(store.accessible(&Session::default(), Mode::Read).is_empty(), "anonymous denied");
}

// 31. `isNoneOf` BRIDGE SHAPE: a single public head carrying one exceptMatcher PER
//     member of the exclusion set (the list-valued `neq` / ACP noneOf dual).
#[test]
fn recipient_isnoneof_emits_one_exception_per_member() {
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &recipient_isnoneof_policy(), &req);
    assert!(out.granted, "isNoneOf maps faithfully to a noneOf condition: {out:?}");
    assert_eq!(
        cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        1,
        "everyone-except is a single public head"
    );
    let ms = except_matchers(&g);
    assert_eq!(ms.len(), 2, "one exceptMatcher per excluded member: {ms:?}");
    assert!(ms.iter().any(|m| m.1.contains("bob.ex")), "bob carved out: {ms:?}");
    assert!(ms.iter().any(|m| m.1.contains("dave.ex")), "dave carved out: {ms:?}");

    // RE-CHECKED end-to-end: everyone reads EXCEPT the set members. The public head
    // matches anonymous too — the accepted sq-5037 everyone-except semantics.
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&recipient_isnoneof_policy(), &req).granted);
    assert!(reads(&mut store, ALICE), "alice (not excluded) granted");
    assert!(reads(&mut store, CAROL), "carol (not excluded) granted");
    assert!(!reads(&mut store, BOB), "bob carved out");
    assert!(!reads(&mut store, DAVE), "dave carved out");
    assert!(
        store.accessible(&Session::default(), Mode::Read).iter().any(|gr| gr.as_str() == N1),
        "anonymous (public) is granted; only the set members are excepted"
    );
}

// 32. PARITY with the evaluator on both operators, over an identified-agent panel:
//     the persisted, re-checked condition must verdict exactly as `evaluate` would.
//     Non-vacuous vs the pre-fix routing: Unmappable would freeze a one-shot verdict
//     scoped to the MATERIALIZING party (deny-all for isAnyOf since alice ∉ set;
//     alice-only for isNoneOf), flipping the members'/non-members' verdicts.
#[test]
fn isanyof_bridge_matches_evaluator_verdicts() {
    assert_bridge_evaluator_parity(&recipient_isanyof_policy());
}

#[test]
fn isnoneof_bridge_matches_evaluator_verdicts() {
    assert_bridge_evaluator_parity(&recipient_isnoneof_policy());
}

// 33. MALFORMED right operands stay fail-closed (Unmappable → one-shot), mirroring the
//     evaluator's own guards — never a persisted condition that widens access.
#[test]
fn isnoneof_nonstring_operand_stays_one_shot_fail_closed() {
    // A numeric or dateTime operand is never satisfied by the evaluator's isNoneOf
    // (set_negation_representable), so a persisted everyone-except grant would WIDEN
    // access. One-shot fallback: evaluate denies the materializer → NOTHING emitted.
    // The dateTime case is the distinguishing one: its lexical form is non-empty, so
    // an (incorrect) lexical set-split would fabricate an exception member and fail
    // open to a public grant — the arm must reject on the VALUE TYPE, not set size.
    for operand in
        ["42", r#""2020-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime>"#]
    {
        let pol = recipient_set_policy("isNoneOf", operand);
        let mut g = pod();
        let req = Request::new(odrl("read")).on(N1).by(ALICE);
        let out = materialize_permission_conditional(&mut g, &pol, &req);
        assert!(!out.granted, "isNoneOf over {operand} grants nothing: {out:?}");
        assert_eq!(cond_grants_for(&g, None), 0, "no condition from operand {operand}");
        assert!(except_matchers(&g).is_empty(), "no exception matcher for {operand}");

        let mut store = PodStore::new(pod());
        assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
        for agent in [ALICE, BOB, CAROL] {
            assert!(!reads(&mut store, agent), "{agent} denied (fail-closed) for {operand}");
        }
    }
}

#[test]
fn isanyof_empty_set_is_unsatisfiable_nothing_materialized() {
    // isAnyOf over the empty set has no member to equal — unsatisfiable for everyone
    // in the evaluator; the bridge must not turn it into any persisted head.
    let pol = recipient_set_policy("isAnyOf", r#""""#);
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(!out.granted, "empty isAnyOf set grants nothing: {out:?}");
    assert_eq!(cond_grants_for(&g, None), 0, "no condition from an empty set");
}

#[test]
fn isnoneof_empty_set_stays_one_shot_no_public_widening() {
    // The DEGENERATE empty exclusion set stays one-shot (conservative): the evaluator
    // vacuously satisfies it for a stated recipient, so the frozen path still grants
    // the materializing party — but the bridge must NOT promote a (likely malformed)
    // empty operand into a bare unconditional re-checked public grant.
    let pol = recipient_set_policy("isNoneOf", r#""""#);
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(out.granted, "vacuous isNoneOf holds for the materializer (frozen): {out:?}");
    assert_eq!(cond_grants_for(&g, None), 0, "no re-checked condition from an empty set");

    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(reads(&mut store, ALICE), "frozen grant scoped to the materializer");
    assert!(!reads(&mut store, CAROL), "no public widening from an empty exclusion set");
}

// 34. A RESERVED-ENCODED member anywhere in the exclusion set sinks the WHOLE rule to
//     one-shot — dropping just that member would re-admit it (fail-open); mirrors the
//     single-value neq guard (test 21) for the list-valued path.
#[test]
fn isnoneof_reserved_member_sinks_whole_rule_to_one_shot() {
    let pol = recipient_set_policy(
        "isNoneOf",
        r#""https://bob.ex/card#me|urn:sparq:pair?agent=x&client=y""#,
    );
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    // One-shot: the evaluator proves isNoneOf for alice (not a member) → a frozen
    // alice-scoped grant; crucially NO public noneOf head is emitted.
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(out.granted, "one-shot grants the (non-excluded) materializer: {out:?}");
    assert_eq!(
        cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "a reserved-encoded exclusion member must not widen to a public grant"
    );
    assert!(except_matchers(&g).is_empty(), "no unenforceable matcher emitted");

    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(!reads(&mut store, CAROL), "no public widening: carol denied");
    assert!(!reads(&mut store, BOB), "bob (excluded member) denied");
}

// 35. The PROHIBITION dual: `recipient isAnyOf <set>` on a prohibition persists one
//     re-checked conditional DENY per member, composing with deny-overrides.
#[test]
fn isanyof_prohibition_persists_conditional_deny_per_member() {
    // A public allow (bare permission via the conditional path → Public head)…
    let permit = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/pub> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] .
"#,
        "turtle",
    )
    .unwrap();
    // …plus a prohibition denying the set {bob, carol}.
    let prohib = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/panyof> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ;
                      odrl:operator odrl:isAnyOf ;
                      odrl:rightOperand "https://bob.ex/card#me|https://carol.ex/card#me" ] ] .
"#,
        "turtle",
    )
    .unwrap();
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission_conditional(&permit, &req).granted);
    let out = store.materialize_odrl_prohibition_conditional(&prohib, &req);
    assert!(out.prohibited, "isAnyOf prohibition maps to per-member deny conditions: {out:?}");

    // Deny-overrides through the real path: the set members lose the public allow.
    assert!(!reads(&mut store, BOB), "bob (in set) denied — deny beats allow");
    assert!(!reads(&mut store, CAROL), "carol (in set) denied — deny beats allow");
    assert!(reads(&mut store, ALICE), "alice (not in set) keeps the public allow");
    assert!(reads(&mut store, DAVE), "dave (not in set) keeps the public allow");
}

// ===========================================================================
// [OPUS-4.8] sq-9n1q4 — a BARE odrl:assignee (the rule PROPERTY, not an
// odrl:assignee CONSTRAINT block) with ZERO constraints must NOT widen to
// auth:Public through the conditional entry points. Regression guard for the
// access-control WIDENING bug: `condition_agents` used to ignore `rule.assignee`
// and default an empty recipient set to auth:Public, so a permission scoped to
// ONE assignee granted EVERYONE (incl. anonymous), and a prohibition scoped to
// one assignee DENIED everyone (over-deny). Both are closed by folding
// `rule.assignee` into the condition head when there is no recipient constraint.
// ===========================================================================

// 30. WIDENING CLOSED (permission): a bare-assignee permission (assignee=alice,
//     ZERO constraints) via the CONDITIONAL entry point grants ONLY alice — bob
//     AND an anonymous session are DENIED accessible()/query_as for the target.
//     Before the fix this granted auth:Public (bob + anonymous read n1).
#[test]
fn bare_assignee_permission_conditional_scopes_to_assignee_not_public() {
    // The rule carries `odrl:assignee alice` as a PROPERTY and no constraint block.
    // (`read_policy()` in this file is exactly this shape.)
    let pol = read_policy();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);

    // Free-function form: the ConditionalGrant head is ALICE, NOT auth:Public.
    let mut g = pod();
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(out.granted, "bare-assignee permission still grants: {out:?}");
    assert_eq!(
        cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "NO auth:Public head — the assignee restriction is honoured"
    );
    assert_eq!(cond_grants_for(&g, Some(ALICE)), 1, "the grant head is scoped to alice");

    // Through the real enforcement path: only alice reads n1.
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(reads(&mut store, ALICE), "the assignee (alice) is granted");
    assert!(!reads(&mut store, BOB), "bob (not the assignee) is DENIED — widening closed");
    assert!(
        store.accessible(&Session::default(), Mode::Read).is_empty(),
        "an anonymous session is DENIED — widening closed"
    );

    // End-to-end through query_as: only alice sees the content.
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    let bob = Session { agent: Some(BOB), client: None, issuer: None, now: None };
    assert_eq!(store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 1);
    assert_eq!(store.query_as(&bob, Mode::Read, sel).unwrap().rows.len(), 0);
    assert_eq!(
        store.query_as(&Session::default(), Mode::Read, sel).unwrap().rows.len(),
        0,
        "anonymous query sees nothing"
    );
}

// 31. WIDENING CLOSED (prohibition dual): a bare-assignee prohibition
//     (assignee=alice, ZERO constraints) via the CONDITIONAL entry point denies
//     ONLY alice — bob keeps a pre-existing public allow. Before the fix this
//     materialized a PUBLIC deny (over-deny: everyone, incl. bob, denied).
#[test]
fn bare_assignee_prohibition_conditional_scopes_to_assignee_not_public() {
    let mut store = PodStore::new(pod());
    // Baseline PUBLIC allow so we can observe the deny biting only the assignee.
    let permit = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pub> a odrl:Set ; odrl:permission [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] ."#,
        "turtle",
    )
    .unwrap();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission_conditional(&permit, &req).granted);
    assert!(reads(&mut store, BOB), "bob reads via the public allow before the deny");

    // A bare-assignee prohibition: `odrl:assignee alice`, ZERO constraints.
    let prohib = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pbare> a odrl:Set ; odrl:prohibition [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ;
            odrl:assignee <https://alice.ex/card#me> ] ."#,
        "turtle",
    )
    .unwrap();

    // Free-function form: the deny head is ALICE, NOT auth:Public.
    let mut g = pod();
    assert!(materialize_permission_conditional(&mut g, &permit, &req).granted);
    let dout = materialize_prohibition_conditional(&mut g, &prohib, &req);
    assert!(dout.prohibited, "bare-assignee prohibition materialises a deny: {dout:?}");
    assert_eq!(
        cond_denies_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "NO auth:Public deny — the deny is scoped to the assignee"
    );
    assert_eq!(cond_denies_for(&g, Some(ALICE)), 1, "the deny head is scoped to alice");

    // Through the real enforcement path: deny-overrides removes ONLY alice's access.
    assert!(store.materialize_odrl_prohibition_conditional(&prohib, &req).prohibited);
    assert!(!reads(&mut store, ALICE), "alice (the assignee) is denied — deny-overrides");
    assert!(reads(&mut store, BOB), "bob keeps the public allow — NOT over-denied");
    assert!(
        !store.accessible(&Session::default(), Mode::Read).is_empty(),
        "an anonymous session keeps the public allow — NOT over-denied"
    );
}

// ===========================================================================
// [OPUS-4.8] sq-izzak — a rule whose ONLY restriction is a COMPOUND
// `odrl:LogicalConstraint` (`odrl:and`/`odrl:or`/`odrl:xone`) with ZERO atomic
// constraints must NOT widen to auth:Public through the conditional entry
// points. Regression guard for the access-control WIDENING bug:
// `map_constraints_to_agents` used to examine ONLY `rule.constraints`, so a rule
// carrying only a compound constraint mapped `Faithful` with an EMPTY recipient
// set → an auth:Public head, silently DROPPING the compound restriction (a
// permission granted EVERYONE incl. anonymous; the prohibition dual over-denied
// everyone). Closed by classifying any `logical_constraints` as Unmappable →
// the one-shot path (which DOES enforce the compound, frozen).
//
// A `recipient eq alice` sub-constraint is used so the one-shot fallback grants/
// denies EXACTLY alice (the evaluator reads Request::party as recipient evidence),
// while bob and an anonymous session are structurally excluded.
// ===========================================================================

/// A permission whose ONLY restriction is a compound `odrl:and` (one operand: a
/// `recipient eq alice` sub-constraint). ZERO atomic `rule.constraints`, NO bare
/// `odrl:assignee` property → the pre-fix conditional path folded an empty
/// recipient set to auth:Public, dropping the compound restriction.
fn compound_recipient_permit_policy() -> sparq_policy::Policy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/cand> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:and
        [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
          odrl:rightOperand <https://alice.ex/card#me> ] ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

// 32. WIDENING CLOSED (permission): a compound-only permission (an `odrl:and`
//     of a single `recipient eq alice`, ZERO atomic constraints) via the
//     CONDITIONAL entry point grants ONLY alice — no auth:Public head, and bob
//     AND an anonymous session are DENIED through accessible()/query_as. Before
//     the fix `map_constraints_to_agents` ignored the compound → Faithful with an
//     empty recipient set → an auth:Public grant (bob + anonymous read n1).
#[test]
fn compound_only_permission_conditional_scopes_not_public() {
    // Sanity: the policy really carries a compound constraint and NO atomic one.
    let pol = compound_recipient_permit_policy();
    assert_eq!(pol.permissions[0].constraints.len(), 0, "no atomic constraint");
    assert_eq!(pol.permissions[0].logical_constraints.len(), 1, "one compound constraint");
    assert!(pol.permissions[0].assignee.is_none(), "no bare assignee property");

    // Free-function form: NO auth:Public head is materialized (the compound forces
    // the one-shot fallback, which freezes a grant scoped to the materializing party).
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(out.granted, "compound-only permission still grants alice one-shot: {out:?}");
    assert_eq!(
        cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "NO auth:Public head — the compound restriction is NOT dropped"
    );

    // Through the real enforcement path: only alice reads n1; bob + anonymous denied.
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(reads(&mut store, ALICE), "the compound recipient (alice) is granted");
    assert!(!reads(&mut store, BOB), "bob (not the recipient) is DENIED — widening closed");
    assert!(
        store.accessible(&Session::default(), Mode::Read).is_empty(),
        "an anonymous session is DENIED — widening closed"
    );

    // End-to-end through query_as: only alice sees the content.
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    let bob = Session { agent: Some(BOB), client: None, issuer: None, now: None };
    assert_eq!(store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 1);
    assert_eq!(store.query_as(&bob, Mode::Read, sel).unwrap().rows.len(), 0);
    assert_eq!(
        store.query_as(&Session::default(), Mode::Read, sel).unwrap().rows.len(),
        0,
        "anonymous query sees nothing"
    );
}

// 33. WIDENING CLOSED (prohibition dual): a compound-only prohibition (an
//     `odrl:xone` of a single `recipient eq alice`, ZERO atomic constraints) via
//     the CONDITIONAL entry point denies ONLY alice — bob AND an anonymous session
//     keep a pre-existing public allow. Before the fix this materialized a PUBLIC
//     deny (over-deny: everyone, incl. bob + anonymous, denied). `odrl:xone` is
//     used to exercise a second combinator (satisfied iff exactly one operand
//     holds — here the single `recipient eq alice`).
#[test]
fn compound_only_prohibition_conditional_scopes_not_public() {
    let mut store = PodStore::new(pod());
    // Baseline PUBLIC allow so we can observe the deny biting only the recipient.
    let permit = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pub> a odrl:Set ; odrl:permission [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] ."#,
        "turtle",
    )
    .unwrap();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission_conditional(&permit, &req).granted);
    assert!(reads(&mut store, BOB), "bob reads via the public allow before the deny");

    // A compound-only prohibition: `odrl:xone` of one `recipient eq alice`, ZERO atomic.
    let prohib = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pxone> a odrl:Set ; odrl:prohibition [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ;
            odrl:constraint [ a odrl:LogicalConstraint ; odrl:xone
                [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
                  odrl:rightOperand <https://alice.ex/card#me> ] ] ] ."#,
        "turtle",
    )
    .unwrap();
    assert_eq!(prohib.prohibitions[0].constraints.len(), 0, "no atomic constraint");
    assert_eq!(prohib.prohibitions[0].logical_constraints.len(), 1, "one compound constraint");

    // Free-function form: NO auth:Public deny head — the deny is scoped to alice
    // (one-shot fallback freezes a deny for the materializing party iff it matches).
    let mut g = pod();
    assert!(materialize_permission_conditional(&mut g, &permit, &req).granted);
    let dout = materialize_prohibition_conditional(&mut g, &prohib, &req);
    assert!(dout.prohibited, "compound-only prohibition materialises a deny for alice: {dout:?}");
    assert_eq!(
        cond_denies_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "NO auth:Public deny — the compound restriction is NOT dropped"
    );

    // Through the real enforcement path: deny-overrides removes ONLY alice's access.
    assert!(store.materialize_odrl_prohibition_conditional(&prohib, &req).prohibited);
    assert!(!reads(&mut store, ALICE), "alice (the recipient) is denied — deny-overrides");
    assert!(reads(&mut store, BOB), "bob keeps the public allow — NOT over-denied");
    assert!(
        !store.accessible(&Session::default(), Mode::Read).is_empty(),
        "an anonymous session keeps the public allow — NOT over-denied"
    );
}

// ===========================================================================
// [FABLE-5] sq-37f1a — an EMPTY static closure must stay MATERIALIZED under the
// bridge feature. The static materializer installs `<urn:sparq:auth>` even when the
// closure grants nothing (presence == the "materialized" marker; empty-but-present
// == a definitive Resolved deny). The post-materialize ledger reconcile
// (`reconcile_bridged_after_static` → `BridgeLedger::refresh`) used to route the
// baseline reset through the drop-when-empty `install_triples`, deleting the marker
// and turning the definitive 403-class deny into a retryable `Unloaded` (a 503 at
// the server) — only in odrl-bridge builds, which is exactly the combined-feature
// breakage issue #2718 pinned at the server level.
// ===========================================================================

/// A pod with a syntactically-valid ACL that GRANTS NOTHING: the `acl:agentGroup`
/// target has no `vcard:hasMember`, so the WAC closure is empty.
fn pod_with_grantless_acl() -> Graph {
    Graph::load_dataset(
        r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n1.acl#rule> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#rule> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#rule> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#rule> <http://www.w3.org/ns/auth/acl#agentGroup> <https://alice.ex/card#me> <https://pod.ex/notes/n1.acl> .
"#,
        "nquads",
    )
    .expect("dataset loads")
}

// 21. An empty WAC closure decides as a DEFINITIVE Resolved deny, not a retryable
//     Unloaded: the bridge reconcile keeps the empty `<urn:sparq:auth>` present.
#[test]
fn empty_static_closure_stays_materialized_as_resolved_deny() {
    let mut store = PodStore::new(pod_with_grantless_acl());
    let stats = store.materialize_wac().expect("materializes");
    assert_eq!(stats.auth_triples, 0, "the closure grants nothing (member-less group)");
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    let d = store.decide(&alice, N1, Mode::Read);
    assert!(!d.allow, "no grant => deny");
    assert_eq!(
        d.status,
        sparq_solid::AclStatus::Resolved,
        "an empty MATERIALIZED view is a definitive Resolved deny (403), never a \
         retryable Unloaded (503) — the ledger reconcile must not drop the marker"
    );
}

// 22. The presence-preserving reset must not INVENT the marker either: a store whose
//     view was never statically materialized stays `Unloaded` through a ledger refresh.
#[test]
fn refresh_does_not_invent_the_materialized_marker() {
    let mut store = PodStore::new(pod_with_grantless_acl());
    // No materialize_* call: the view is absent. A bare refresh has nothing to replay
    // and must not install an empty `<urn:sparq:auth>` shell.
    assert_eq!(store.refresh_odrl_grants(), 0, "empty ledger retracts nothing");
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    let d = store.decide(&alice, N1, Mode::Read);
    assert!(!d.allow, "fail-closed: deny");
    assert_eq!(
        d.status,
        sparq_solid::AclStatus::Unloaded,
        "never-materialized stays a retryable Unloaded — refresh must not fake the marker"
    );
}

// 23. Nor may the marker survive via a BRIDGED-ONLY grant: without any static
//     materialization the bridged grant alone creates `<urn:sparq:auth>`, so the
//     graph's presence at refresh-time does NOT mean a static closure was computed.
//     Once the grant is withdrawn and replay emits nothing, the view must go ABSENT
//     again (`static_baseline` was never captured) — the status returns to a
//     retryable `Unloaded`, never an invented "materialized, no grants" Resolved deny.
#[test]
fn bridged_only_retraction_returns_to_unloaded() {
    let mut store = PodStore::new(pod_with_grantless_acl());
    // NO static materialize_* call — the baseline is never captured; the bridged
    // grant is what creates the auth view.
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission(&read_policy(), &req).granted);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.decide(&alice, N1, Mode::Read).allow, "bridged grant is live");

    // The policy WITHDRAWS the permission → refresh replays it to nothing.
    let (matched, retracted) =
        store.refresh_odrl_grant(&empty_policy(), &req, BridgeKind::Permission);
    assert!(matched, "the tracked grant slot matched");
    assert_eq!(retracted, 1, "the withdrawn grant was retracted");

    let d = store.decide(&alice, N1, Mode::Read);
    assert!(!d.allow, "fail-closed: deny");
    assert_eq!(
        d.status,
        sparq_solid::AclStatus::Unloaded,
        "a never-statically-materialized store returns to retryable Unloaded once its \
         only bridged grant retracts — refresh must not preserve an empty view no \
         static baseline was ever captured for"
    );
}

// ===========================================================================
// [SONNET-4.6] sq-rf9uv — PartyCollection-AWARE conditional heads. The evaluator
// matches a collection-valued `odrl:assignee`/`odrl:recipient` by identity-OR-membership,
// but an ACP `auth:agent` head matches by IDENTITY alone (a session carries no membership
// evidence). So the persisted head must be expanded to the members the request evidenced
// — on the ALLOW side only. Every other collection-valued head (a DENY head, an ALLOW's
// `noneOf` carve-out, an ALLOW head with no evidenced member) leaves the frozen path
// entirely: an identity-matched head cannot re-check membership, so freezing one either
// lets members ESCAPE the restriction (fail-open) or binds nobody at all.
//
// Collection IDENTITY is read from the POLICY (`Policy::party_collections`, retained by
// the parser from `a odrl:PartyCollection` / `odrl:partOf`) as well as from the request's
// membership evidence, so a collection with ZERO supplied member edges is recognised —
// that is what stops a bare collection IRI being frozen as if it were a plain party.
// ===========================================================================

/// The `odrl:PartyCollection` alice and bob are members of (carol is not).
const LAB: &str = "https://pod.ex/groups/lab";

/// The `a odrl:PartyCollection` declaration that gives LAB its identity in the policy
/// document, independently of any membership edge.
const LAB_DECL: &str = "<https://pod.ex/groups/lab> a odrl:PartyCollection .";

/// "the LAB collection MAY read n1" — a bare `odrl:assignee` naming a PartyCollection.
fn lab_assignee_permit() -> sparq_policy::Policy {
    parse_policy_str(
        &format!(
            r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
{}
<urn:pol/lab> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://pod.ex/groups/lab> ] .
"#,
            LAB_DECL
        ),
        "turtle",
    )
    .expect("policy parses")
}

/// "the LAB collection MUST NOT read n1" — a prohibition whose head is the collection.
fn lab_prohibition() -> sparq_policy::Policy {
    parse_policy_str(
        &format!(
            r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
{}
<urn:pol/labdeny> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://pod.ex/groups/lab> ] .
"#,
            LAB_DECL
        ),
        "turtle",
    )
    .expect("policy parses")
}

/// "anyone EXCEPT the LAB collection MAY read n1" — an ALLOW `neq` carve-out whose
/// excluded value is the collection.
fn lab_carve_out_permit() -> sparq_policy::Policy {
    parse_policy_str(
        &format!(
            r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
{}
<urn:pol/notlab> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
                      odrl:rightOperand <https://pod.ex/groups/lab> ] ] .
"#,
            LAB_DECL
        ),
        "turtle",
    )
    .expect("policy parses")
}

/// "everyone MAY read n1" — the public allow used to show a prohibition biting.
fn public_read_permit() -> sparq_policy::Policy {
    parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pub2> a odrl:Set ; odrl:permission [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] ."#,
        "turtle",
    )
    .expect("policy parses")
}

/// The membership evidence "alice and bob are `odrl:partOf` the LAB collection".
fn lab_members(req: Request) -> Request {
    req.with_party_memberships([(ALICE, LAB), (BOB, LAB)])
}

// 40. THE BUG: a collection-valued assignee emitted ONE head (the collection IRI), which
//     no member session can ever match — the conditional grant was dead and every member
//     was over-restricted. Now each KNOWN member gets a head and reads through the real
//     enforcement path; a non-member still does not.
#[test]
fn collection_assignee_grants_one_head_per_known_member() {
    let mut g = pod();
    let req = lab_members(Request::new(odrl("read")).on(N1).by(ALICE));
    let out = materialize_permission_conditional(&mut g, &lab_assignee_permit(), &req);
    assert!(out.granted, "a collection assignee still maps to a condition: {out:?}");
    assert_eq!(cond_grants_for(&g, Some(ALICE)), 1, "member alice gets a head");
    assert_eq!(cond_grants_for(&g, Some(BOB)), 1, "member bob gets a head");
    assert_eq!(cond_grants_for(&g, Some(LAB)), 1, "the collection IRI stays a head too");
    assert_eq!(cond_grants_for(&g, Some(CAROL)), 0, "a non-member is NOT granted");

    // RE-CHECK through the real enforcement path.
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&lab_assignee_permit(), &req).granted);
    assert!(reads(&mut store, ALICE), "member alice reads");
    assert!(reads(&mut store, BOB), "member bob reads — even though ALICE materialized");
    assert!(!reads(&mut store, CAROL), "non-member denied");
    assert!(store.accessible(&Session::default(), Mode::Read).is_empty(), "anonymous denied");
}

// 41. SOUNDNESS FLOOR: the expansion draws ONLY on the caller-supplied membership
//     evidence, so a ZERO-EDGE collection has nothing to expand to. The policy declares
//     LAB a collection, so the head is recognised as one and the rule leaves the frozen
//     path entirely rather than persist a bare collection IRI that binds nobody — no
//     conditional grant at all, and no party granted on an UNPROVEN membership.
#[test]
fn collection_assignee_without_evidence_emits_no_conditional_grant() {
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE); // no membership evidence
    let out = materialize_permission_conditional(&mut g, &lab_assignee_permit(), &req);
    assert!(!out.granted, "a zero-edge collection assignee grants nothing: {out:?}");
    assert_eq!(cond_grants_for(&g, None), 0, "NO conditional grant is persisted");
    assert_eq!(cond_grants_for(&g, Some(LAB)), 0, "not even the bare collection IRI");
    assert_eq!(cond_grants_for(&g, Some(ALICE)), 0, "no head on an unproven membership");

    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission_conditional(&lab_assignee_permit(), &req).granted);
    assert!(!reads(&mut store, ALICE), "unproven membership grants nothing");
    assert!(!reads(&mut store, BOB), "unproven membership grants nothing");
    assert!(!reads(&mut store, CAROL), "and nothing widened to a non-member");
}

// 42. THE DENY DUAL STAYS ONE-SHOT: a collection-valued deny head cannot re-check
//     membership, so freezing it to the evidenced members would let every UNLISTED member
//     escape the prohibition (fail-OPEN). No deny CONDITION is emitted; the one-shot
//     evaluator — which does the real identity-or-membership check — denies instead.
#[test]
fn collection_prohibition_stays_one_shot() {
    let prohib = lab_prohibition();
    let req = lab_members(Request::new(odrl("read")).on(N1).by(ALICE));

    let mut g = pod();
    let out = materialize_prohibition_conditional(&mut g, &prohib, &req);
    assert!(out.prohibited, "the member's deny still materializes: {out:?}");
    assert_eq!(cond_denies_for(&g, None), 0, "NO re-checked deny condition for a collection");
    assert_eq!(
        out.deny_triple.as_ref().map(|t| (t.0.as_str(), t.1.contains("denyRead"))),
        Some((ALICE, true)),
        "the frozen one-shot deny names the requesting member: {out:?}"
    );

    // End-to-end with deny-overrides: a public allow is in force; each member's own
    // request materializes that member's frozen deny (the one-shot contract).
    let permit = public_read_permit();
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&permit, &req).granted);
    assert!(reads(&mut store, ALICE), "public allow in force before the deny");
    assert!(store.materialize_odrl_prohibition_conditional(&prohib, &req).prohibited);
    assert!(!reads(&mut store, ALICE), "DENY-OVERRIDES: the member loses access");
    let bob_req = lab_members(Request::new(odrl("read")).on(N1).by(BOB));
    assert!(store.materialize_odrl_prohibition_conditional(&prohib, &bob_req).prohibited);
    assert!(!reads(&mut store, BOB), "bob's own request denies bob");
    assert!(reads(&mut store, CAROL), "a non-member keeps the public allow");
}

// 43. AN ALLOW'S CARVE-OUT NAMING A COLLECTION ALSO STAYS ONE-SHOT: a frozen ACP
//     `noneOf` matcher carries the collection IRI, which matches no member session, so a
//     member of the EXCLUDED collection would sail past the exception and keep access
//     (fail-OPEN — the widening the bridge exists to prevent). The one-shot path's
//     evaluator applies the real `neq`-over-membership check instead.
#[test]
fn collection_carve_out_stays_one_shot() {
    let pol = lab_carve_out_permit();
    // carol (NOT a lab member) asks; the evidence names bob as a lab member.
    let req = Request::new(odrl("read")).on(N1).by(CAROL).with_party_membership(BOB, LAB);

    let mut g = pod();
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(out.granted, "carol is outside the excluded collection: {out:?}");
    assert_eq!(cond_grants_for(&g, None), 0, "a collection carve-out emits NO condition");

    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(reads(&mut store, CAROL), "the un-excluded requester reads (frozen)");
    assert!(!reads(&mut store, BOB), "a member of the EXCLUDED collection must NOT read");
    assert!(!reads(&mut store, ALICE), "no widening to unrelated agents");
}

// 44. THE ZERO-EDGE DENY IS CLOSED: a prohibition naming a collection the request
//     supplied NO `odrl:partOf` edge for used to take the ordinary concrete-head path and
//     persist the bare collection IRI as a deny head — a head no member session can match,
//     so every member escaped the prohibition. Collection identity now comes from the
//     policy's own `a odrl:PartyCollection` declaration, so the rule leaves the frozen path
//     regardless of evidence: NO conditional deny is emitted, and nothing is widened.
#[test]
fn zero_edge_collection_prohibition_emits_no_conditional_deny() {
    let prohib = lab_prohibition();
    // NO `.with_party_membership(…)`: identity comes from the policy declaration alone.
    let req = Request::new(odrl("read")).on(N1).by(CAROL);

    let mut g = pod();
    let out = materialize_prohibition_conditional(&mut g, &prohib, &req);
    assert_eq!(cond_denies_for(&g, None), 0, "NO re-checked deny condition: {out:?}");
    assert_eq!(
        cond_denies_for(&g, Some(LAB)),
        0,
        "and specifically NOT the bare collection IRI, which binds no member session"
    );

    // Through the real enforcement path, against a public allow: the collection deny is
    // never persisted as an unenforceable head, and a member who DOES evidence membership
    // is bound by the one-shot deny the fallback materializes.
    let permit = public_read_permit();
    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&permit, &req).granted);
    assert!(!store.materialize_odrl_prohibition_conditional(&prohib, &req).prohibited,
        "carol is not a member under any evidence, so no deny is materialized for her");
    assert!(reads(&mut store, CAROL), "the non-member keeps the public allow");
    let alice_req = lab_members(Request::new(odrl("read")).on(N1).by(ALICE));
    assert!(store.materialize_odrl_prohibition_conditional(&prohib, &alice_req).prohibited);
    assert!(!reads(&mut store, ALICE), "the member is bound by the one-shot deny");
}

// 45. THE ZERO-EDGE CARVE-OUT IS CLOSED — the case that actually WIDENED access. With no
//     `odrl:partOf` edge the `neq` carve-out used to freeze an `auth:Public` grant plus a
//     `noneOf` matcher naming the collection; the matcher accepts no member session, so
//     every member of the EXCLUDED collection kept the public grant. The policy-declared
//     identity now routes the rule to the one-shot path: no public conditional grant is
//     persisted, and a member of the excluded collection cannot read.
#[test]
fn zero_edge_collection_carve_out_emits_no_conditional_grant() {
    let pol = lab_carve_out_permit();
    let req = Request::new(odrl("read")).on(N1).by(CAROL); // no membership evidence

    let mut g = pod();
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(out.granted, "carol is outside the excluded collection: {out:?}");
    assert_eq!(
        cond_grants_for(&g, None),
        0,
        "NO everyone-except condition is persisted on an unenforceable collection matcher"
    );

    let mut store = PodStore::new(pod());
    assert!(store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(reads(&mut store, CAROL), "the un-excluded requester reads (frozen one-shot)");
    assert!(!reads(&mut store, BOB), "a member of the EXCLUDED collection must NOT read");
    assert!(!reads(&mut store, ALICE), "nor any other member");
}

// 46. IDENTITY WITHOUT A TYPE TRIPLE: a document that states `<alice> odrl:partOf <lab>`
//     has identified `<lab>` as a collection just as surely as `a odrl:PartyCollection`,
//     and the parser retains both. The prohibition still leaves the frozen path even
//     though the REQUEST carries no membership evidence at all.
#[test]
fn policy_stated_membership_edge_identifies_the_collection() {
    let prohib = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<https://pod.ex/alice> odrl:partOf <https://pod.ex/groups/lab> .
<urn:pol/labdeny3> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://pod.ex/groups/lab> ] .
"#,
        "turtle",
    )
    .expect("policy parses");
    assert!(
        prohib.party_collections.contains(LAB),
        "the `odrl:partOf` object is retained as a collection: {:?}",
        prohib.party_collections
    );

    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(CAROL); // no request-side evidence
    materialize_prohibition_conditional(&mut g, &prohib, &req);
    assert_eq!(cond_denies_for(&g, Some(LAB)), 0, "no bare collection deny head");
    assert_eq!(cond_denies_for(&g, None), 0, "no conditional deny at all");
}
