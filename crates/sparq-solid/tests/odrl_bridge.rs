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
fn read_policy() -> sparq_policy::ValidatedPolicy {
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
fn write_prohibition() -> sparq_policy::ValidatedPolicy {
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

    // (d) Partyless prohibition (no assignee) matched by an anonymous request: there is
    //     no party to freeze the deny to, so it denies every session on the target.
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
    let out4 = materialize_prohibition(&mut g4, &any_prohib, &no_party);
    assert!(out4.prohibited, "{out4:?}");
    assert_eq!(cond_denies_for(&g4, Some("https://sparq.dev/ns/auth#Public")), 1);

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
fn recipient_policy() -> sparq_policy::ValidatedPolicy {
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

// 16. A purpose-gated permission is decided but never stored: the stored allow does not
//     record the purpose, so a later request with another purpose would ride it. No
//     ConditionalGrant is persisted either.
#[test]
fn purpose_constraint_is_never_stored() {
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

    let req = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .with(odrl("purpose"), Value::Str("research".to_owned()));
    assert!(sparq_policy::evaluate(&pol, &req).allow, "the decision itself allows");
    let mut g = pod();
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(!out.granted, "a purpose-scoped grant is not stored: {out:?}");
    assert_eq!(cond_grants_for(&g, None), 0, "purpose must NOT become a re-checked condition");

    let mut store = PodStore::new(pod());
    store.materialize_odrl_permission_conditional(&pol, &req);
    assert!(!reads(&mut store, ALICE), "nothing stored for alice");
    assert!(!reads(&mut store, CAROL), "no widening");
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

// ===========================================================================
// [OPUS-4.8] sq-dpk4 — refresh / REVOCATION of bridged ODRL grants when the
// underlying ODRL policy changes. SECURITY-SENSITIVE: a withdrawn permission, a
// lapsed time window, or a re-evaluation that now Denies must LOSE access; a static
// WAC/ACP grant must NEVER be dropped; a still-valid bridged grant must survive.
// All assertions go through the REAL enforcement path (accessible / query_as).
// ===========================================================================
use sparq_solid::BridgeKind;

/// alice MAY read n1 ONLY until 2026-01-01 (a time-windowed permission).
fn windowed_read_policy() -> sparq_policy::ValidatedPolicy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/win> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gteq ;
                      odrl:rightOperand "2025-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

/// A policy that grants NOTHING (the permission has been WITHDRAWN entirely).
fn empty_policy() -> sparq_policy::ValidatedPolicy {
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

// 21. A CLOCK-BOUNDED GRANT IS NOT STORED: the permission holds until 2026, so a stored
//     triple (never re-checked against the clock) would outlive it. decide() grants the
//     in-window request, but the bridge stores nothing.
#[test]
fn a_clock_bounded_grant_is_not_stored() {
    let pol = parse_policy_str(
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
    .expect("policy parses");
    let in_window = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .with(odrl("dateTime"), Value::DateTime("2025-06-01T00:00:00Z".to_owned()));
    assert!(sparq_policy::evaluate(&pol, &in_window).allow, "decide grants in the window");
    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission(&pol, &in_window).granted);
    assert!(!store.materialize_odrl_policy(&pol, &in_window).granted);
    assert!(!reads(&mut store, ALICE), "nothing stored for alice");
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

// 27. A windowed grant is never stored, so a refresh with no dateTime evidence has
//     nothing stale to leave behind.
#[test]
fn a_windowed_grant_is_never_stored() {
    let mut store = PodStore::new(pod());
    let pol = windowed_read_policy();
    let in_window = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .with(odrl("dateTime"), Value::DateTime("2025-06-01T00:00:00Z".to_owned()));
    assert!(sparq_policy::evaluate(&pol, &in_window).allow, "the decision allows");
    assert!(!store.materialize_odrl_permission(&pol, &in_window).granted);
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    assert!(store.accessible(&alice, Mode::Read).is_empty());

    let no_evidence = Request::new(odrl("read")).on(N1).by(ALICE);
    let (_, retracted) = store.refresh_odrl_grant(&pol, &no_evidence, BridgeKind::Permission);
    assert_eq!(retracted, 0, "nothing was stored");
    assert!(store.accessible(&alice, Mode::Read).is_empty());
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
fn windowed_write_prohibition() -> sparq_policy::ValidatedPolicy {
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
fn empty_prohibition_policy() -> sparq_policy::ValidatedPolicy {
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
// enforcement path (PodStore::accessible / query_as). A purpose-gated permission is
// never stored (the stored allow has no purpose, so it would outlive the request);
// a mismatch or MISSING purpose grants nothing; the prohibition dual carves out only on
// a matching stated purpose, and a missing purpose does NOT withdraw the carve-out.
// Match is exact (no hierarchy). All assertions go through accessible / query_as.
// ===========================================================================

const RESEARCH: &str = "urn:purpose/research";
const MARKETING: &str = "urn:purpose/marketing";

/// alice MAY read n1, gated on purpose = research (exact IRI).
fn purpose_read_policy() -> sparq_policy::ValidatedPolicy {
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

// 28. purpose MATCH is allowed but not stored; mismatch denies.
#[test]
fn purpose_match_is_never_stored() {
    let pol = purpose_read_policy();
    // [OPUS-4.8] sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };

    // (a) Matching purpose → the decision allows, but the stored view has no purpose,
    //     so nothing is stored (a later marketing request would otherwise ride it).
    let mut store = PodStore::new(pod());
    let ok = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .for_purpose(Value::Iri(RESEARCH.to_owned()));
    assert!(sparq_policy::evaluate(&pol, &ok).allow, "matching purpose is allowed");
    assert!(!store.materialize_odrl_permission(&pol, &ok).granted, "but never stored");
    assert!(!reads_n1(&mut store, ALICE), "no purpose-free read for alice");
    assert_eq!(store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 0);

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

    // (c) NO purpose stated → the carve-out is *unknown*, and a prohibition fires
    //     unless it definitely does not apply (the matched_prohibition boundary), so the
    //     deny is materialized: the request might be for the banned purpose.
    let mut store3 = PodStore::new(pod());
    assert!(store3.materialize_odrl_permission(&permit, &unconstrained).granted);
    let out3 = store3.materialize_odrl_prohibition(&prohib, &unconstrained);
    assert!(out3.prohibited, "no purpose evidence → the carve-out still applies: {out3:?}");
    assert!(!reads_n1(&mut store3, ALICE), "deny-overrides on an unknown purpose");
}

// ===========================================================================
// [OPUS-4.8] sq-5037 — `odrl:recipient neq X` / "everyone-except-X" → an ACP
// `noneOf` exception: the grant head is auth:Authenticated with an auth:exceptMatcher
// carving out X. RE-CHECKED per session: every identified agent reads EXCEPT X.
// Anonymous is denied, since an exclusion cannot be checked without an identity.
// ===========================================================================

const DAVE: &str = "https://dave.ex/card#me";

/// "everyone EXCEPT bob may read n1" — recipient neq bob.
fn recipient_neq_policy() -> sparq_policy::ValidatedPolicy {
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
    // alice (≠ the reserved party) is allowed, but a constrained grant is not stored.
    // Crucially NO public noneOf grant is emitted — the reserved exclusion cannot
    // become a matcher, so access is NOT widened to everyone-except.
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(!out.granted, "a constrained grant is not stored: {out:?}");
    assert_eq!(
        cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "reserved-encoded neq must NOT widen to a public everyone-except grant"
    );
    assert!(except_matchers(&g).is_empty(), "no unenforceable matcher emitted");

    // Nothing stored for anyone: a stranger is denied (no widening to public).
    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(!reads(&mut store, ALICE), "nothing stored for alice either");
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
fn recipient_neq_policy_excluding(excluded: &str) -> sparq_policy::ValidatedPolicy {
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

// ===========================================================================
// [OPUS-4.8] sq-5037 follow-up — COMBINED recipient `eq A AND neq B` in ONE rule
// through the BRIDGE. The bridge emits `agents=[A]` (positive head) + `except=[B]`
// (a noneOf exceptMatcher) — both on the SAME ConditionalGrant. Structurally emitted
// by sq-5037 but untested at the bridge level (the per-head exception path).
// ===========================================================================

/// "carol (and only carol), but never bob, may read n1" — recipient eq carol AND neq bob.
fn recipient_eq_and_neq_policy() -> sparq_policy::ValidatedPolicy {
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

// ===========================================================================
// [OPUS-4.8] sq-4r70 — CONSTRAINT-CONDITIONAL DENY: an ACP-style deny that is
// conditional on a recipient/assignee constraint (the dual of the conditional grant).
// Materialized as a re-checked `auth:ConditionalGrant` with `auth:effect auth:Deny`,
// composing with deny-overrides. The deny appears/retracts as the condition flips.
// ===========================================================================
use sparq_solid::materialize_prohibition_conditional;

/// Give every session read access to n1 through a static WAC rule, for the deny tests
/// that layer a deny over a public allow. `permit` is the unconstrained ODRL permission
/// the allow stands for; the bridge stores one-shot grants only, never a public one.
fn install_public_read(store: &mut PodStore, permit: &sparq_policy::ValidatedPolicy) {
    assert!(permit.prohibitions.is_empty());
    assert!(permit.permissions.iter().all(|r| {
        r.constraints.is_empty() && r.logical_constraints.is_empty() && r.assignee.is_none()
    }));
    let nq = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#agentClass> <http://xmlns.com/foaf/0.1/Agent> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n1.acl> .
"#;
    *store = PodStore::new(Graph::load_dataset(nq, "nquads").expect("pod loads"));
    store.materialize_wac().expect("wac");
}

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

/// "carol (recipient) is PROHIBITED from reading n1" — a recipient-eq prohibition. The
/// conditional path does not re-check the recipient per session yet (#6743), so it
/// denies every session on n1.
fn prohibit_recipient_carol_policy() -> sparq_policy::ValidatedPolicy {
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
//     ConditionalGrant with auth:effect auth:Deny on auth:Public, whoever materialized it.
#[test]
fn conditional_deny_emits_deny_effect_condition() {
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_prohibition_conditional(&mut g, &prohibit_recipient_carol_policy(), &req);
    assert!(out.prohibited, "the recipient prohibition maps to a deny condition: {out:?}");
    assert_eq!(cond_denies_for(&g, Some("https://sparq.dev/ns/auth#Public")), 1, "one deny on auth:Public");
    assert_eq!(cond_denies_for(&g, Some(CAROL)), 0, "no per-party head");
    // It is a DENY, not an allow (the audit anchor reports the effect predicate).
    assert_eq!(out.deny_triple.as_ref().map(|t| t.1.as_str()),
        Some("https://sparq.dev/ns/auth#effect"), "deny anchor: {out:?}");
}

// 25. DENY-OVERRIDES end-to-end: a public allow grant is in force, and a prohibition on
//     carol is layered over it. carol is denied (deny beats allow). Until #6743 re-checks
//     the recipient per request, bob is denied too: a documented over-deny.
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
    install_public_read(&mut store, &permit);
    assert!(reads(&mut store, CAROL), "everyone reads before the deny");
    assert!(reads(&mut store, BOB), "bob reads before the deny");

    // Now layer the conditional deny carving out carol.
    let out = store.materialize_odrl_prohibition_conditional(&prohibit_recipient_carol_policy(), &req);
    assert!(out.prohibited, "deny condition materialized: {out:?}");
    assert!(!reads(&mut store, CAROL), "DENY-OVERRIDES: carol loses access");
    assert!(!reads(&mut store, BOB), "bob is over-denied until #6743");
    // End-to-end query_as: carol sees nothing, bob sees the content.
    // [OPUS-4.8] sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let carol = Session { agent: Some(CAROL), client: None, issuer: None, now: None };
    let bob = Session { agent: Some(BOB), client: None, issuer: None, now: None };
    assert_eq!(store.query_as(&carol, Mode::Read, sel).unwrap().rows.len(), 0);
    assert_eq!(store.query_as(&bob, Mode::Read, sel).unwrap().rows.len(), 0);
}

// 25b. An `odrl:assignee` CONSTRAINT has no evidence in a request, so `decide` keeps the
//      prohibition in force for every party. The conditional path must not narrow it to
//      an alice-only head: bob, asking with no assignee context, loses his public WAC
//      read, and keeps losing it after a ledger refresh.
#[test]
fn assignee_constrained_prohibition_denies_everyone_decide_denies() {
    let mut store = PodStore::new(pod());
    let permit = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pub> a odrl:Set ; odrl:permission [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] ."#,
        "turtle",
    )
    .unwrap();
    install_public_read(&mut store, &permit);
    let prohib = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pa> a odrl:Set ; odrl:prohibition [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ;
            odrl:constraint [ odrl:leftOperand odrl:assignee ; odrl:operator odrl:eq ;
                              odrl:rightOperand <https://alice.ex/card#me> ] ] ."#,
        "turtle",
    )
    .unwrap();
    let req = Request::new(odrl("read")).on(N1).by(BOB);
    assert!(!sparq_policy::decide(&prohib, &req).allow);
    assert!(sparq_policy::matched_prohibition(&prohib, &req).is_some());
    assert!(reads(&mut store, BOB), "bob reads through the public grant first");

    let out = store.materialize_odrl_prohibition_conditional(&prohib, &req);
    assert!(out.prohibited, "{out:?}");
    assert_eq!(cond_denies_for(&store.graph, Some(ALICE)), 0, "no alice-only head");
    assert!(!reads(&mut store, BOB), "bob is denied, as decide denies him");
    store.refresh_odrl_grants();
    assert!(!reads(&mut store, BOB), "the deny survives a ledger refresh");
}

// 26. The deny APPEARS/RETRACTS: a recipient-neq prohibition denies n1 (for everyone
//     until #6743); when the prohibition is WITHDRAWN, refresh retracts the deny and
//     access is restored (composes with sq-2pcf deny-retraction).
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
    install_public_read(&mut store, &permit);

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
    assert!(!reads(&mut store, CAROL), "carol (not bob) is denied by the conditional deny");
    assert!(!reads(&mut store, BOB), "bob is over-denied until #6743");

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
    assert!(reads(&mut store, BOB), "and so does bob");
    assert_eq!(cond_denies_for(&store.graph, None), 0, "no residual deny condition");
}

// 27. A constraint a session cannot carry (here a dateTime bound; ACP has no clock)
//     turns the prohibition into an unconditional deny for every session.
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
    assert!(out.prohibited, "a deny materializes: {out:?}");
    // A session carries no clock, so the time bound becomes an unconditional deny.
    assert_eq!(cond_denies_for(&g, None), 1, "one deny head: {out:?}");
    assert_eq!(cond_denies_for(&g, Some("https://sparq.dev/ns/auth#Public")), 1, "on auth:Public");
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
fn conflicting_write_policy(conflict_clause: &str) -> sparq_policy::ValidatedPolicy {
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
fn recipient_isanyof_policy() -> sparq_policy::ValidatedPolicy {
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
fn recipient_isnoneof_policy() -> sparq_policy::ValidatedPolicy {
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
fn recipient_set_policy(operator: &str, right_operand: &str) -> sparq_policy::ValidatedPolicy {
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

/// Bridge ↔ evaluator PARITY over identified recipients: each agent's request,
/// materialized on its own, stores a grant for that agent exactly when `decide` grants
/// a lasting permit, never when `evaluate` denies, and never for any other agent or an
/// anonymous session.
fn assert_bridge_evaluator_parity(pol: &sparq_policy::ValidatedPolicy) {
    for agent in [ALICE, BOB, CAROL, DAVE] {
        let req = Request::new(odrl("read")).on(N1).by(agent);
        let mut store = PodStore::new(pod());
        store.materialize_odrl_permission_conditional(pol, &req);
        let stored = sparq_policy::decide(pol, &req).permit.is_some_and(|p| p.lasting());
        assert_eq!(reads(&mut store, agent), stored, "bridge/decide parity for {agent}");
        assert!(stored <= sparq_policy::evaluate(pol, &req).allow, "{agent}: never wider");
        for other in [ALICE, BOB, CAROL, DAVE].into_iter().filter(|o| *o != agent) {
            assert!(!reads(&mut store, other), "{agent}'s grant reaches {other}");
        }
        assert!(store.accessible(&Session::default(), Mode::Read).is_empty(), "anonymous");
    }
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

#[test]
fn identity_constrained_permissions_are_never_stored() {
    for pol in [
        recipient_policy(),
        recipient_neq_policy(),
        recipient_neq_policy_excluding(DAVE),
        recipient_eq_and_neq_policy(),
    ] {
        assert_bridge_evaluator_parity(&pol);
        for agent in [ALICE, BOB, CAROL, DAVE] {
            let req = Request::new(odrl("read")).on(N1).by(agent);
            let mut store = PodStore::new(pod());
            assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
        }
    }
    // The control: an unconstrained grant assigned to alice is stored for her alone.
    let to_alice = parse_policy_str(
        &format!(
            "@prefix odrl: <http://www.w3.org/ns/odrl/2/> . <urn:pol/a> a odrl:Set ; \
             odrl:permission [ odrl:action odrl:read ; odrl:target <{N1}> ; \
             odrl:assignee <{ALICE}> ] ."
        ),
        "turtle",
    )
    .unwrap();
    assert_bridge_evaluator_parity(&to_alice);
    let mut store = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(store.materialize_odrl_permission_conditional(&to_alice, &req).granted);
    assert!(reads(&mut store, ALICE));
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
    // vacuously satisfies it for a stated recipient, so the decision allows
    // the materializing party (nothing is stored) — but the bridge must NOT promote a (likely malformed)
    // empty operand into a bare unconditional re-checked public grant.
    let pol = recipient_set_policy("isNoneOf", r#""""#);
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(!out.granted, "a constrained grant is not stored: {out:?}");
    assert_eq!(cond_grants_for(&g, None), 0, "no re-checked condition from an empty set");

    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(!reads(&mut store, ALICE), "nothing stored for the materializer");
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
    // The evaluator proves isNoneOf for alice (not a member), but a constrained grant
    // is not stored; crucially NO public noneOf head is emitted.
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(!out.granted, "a constrained grant is not stored: {out:?}");
    assert_eq!(
        cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "a reserved-encoded exclusion member must not widen to a public grant"
    );
    assert!(except_matchers(&g).is_empty(), "no unenforceable matcher emitted");

    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(!reads(&mut store, ALICE), "nothing stored for alice");
    assert!(!reads(&mut store, CAROL), "no public widening: carol denied");
    assert!(!reads(&mut store, BOB), "bob (excluded member) denied");
}

// 35. The PROHIBITION dual: `recipient isAnyOf <set>` on a prohibition persists an
//     unconditional deny (the members, and until #6743 everyone else), composing with
//     deny-overrides.
#[test]
fn isanyof_prohibition_conditional_denies_the_target() {
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
    install_public_read(&mut store, &permit);
    let out = store.materialize_odrl_prohibition_conditional(&prohib, &req);
    assert!(out.prohibited, "the isAnyOf prohibition maps to a deny: {out:?}");

    // Deny-overrides through the real path: the set members lose the public allow.
    assert!(!reads(&mut store, BOB), "bob (in set) denied — deny beats allow");
    assert!(!reads(&mut store, CAROL), "carol (in set) denied — deny beats allow");
    assert!(!reads(&mut store, ALICE), "alice (not in set) is over-denied until #6743");
    assert!(!reads(&mut store, DAVE), "dave (not in set) is over-denied until #6743");
}

// ===========================================================================
// [OPUS-4.8] sq-9n1q4 — a BARE odrl:assignee (the rule PROPERTY, not an
// odrl:assignee CONSTRAINT block) with ZERO constraints must NOT widen to
// auth:Public through the conditional entry points. Regression guard for the
// access-control WIDENING bug: `condition_agents` used to ignore `rule.assignee`
// and default an empty recipient set to auth:Public, so a permission scoped to
// ONE assignee granted EVERYONE (incl. anonymous), and a prohibition scoped to
// one assignee DENIED everyone (over-deny). The permission side is closed: a constrained
// grant is not stored. The prohibition side denies everyone on purpose until #6743
// re-checks the party per request.
// ===========================================================================

// 31. A bare-assignee prohibition (assignee=alice, ZERO constraints) via the
//     CONDITIONAL entry point denies alice. Until #6743 re-checks the party per
//     request, it denies every other session on n1 too: a documented over-deny.
#[test]
fn bare_assignee_prohibition_conditional_denies_the_target() {
    let mut store = PodStore::new(pod());
    // Baseline PUBLIC allow so we can observe the deny biting.
    let permit = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pub> a odrl:Set ; odrl:permission [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] ."#,
        "turtle",
    )
    .unwrap();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    install_public_read(&mut store, &permit);
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

    // Free-function form: one unconditional deny on auth:Public, no per-party head.
    let mut g = pod();
    let dout = materialize_prohibition_conditional(&mut g, &prohib, &req);
    assert!(dout.prohibited, "bare-assignee prohibition materialises a deny: {dout:?}");
    assert_eq!(cond_denies_for(&g, Some("https://sparq.dev/ns/auth#Public")), 1);
    assert_eq!(cond_denies_for(&g, Some(ALICE)), 0, "no per-party head");

    // Through the real enforcement path: every session loses access.
    assert!(store.materialize_odrl_prohibition_conditional(&prohib, &req).prohibited);
    assert!(!reads(&mut store, ALICE), "alice (the assignee) is denied — deny-overrides");
    assert!(!reads(&mut store, BOB), "bob is over-denied until #6743");
    assert!(
        store.accessible(&Session::default(), Mode::Read).is_empty(),
        "an anonymous session is denied too"
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
fn compound_recipient_permit_policy() -> sparq_policy::ValidatedPolicy {
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

    // Free-function form: NO auth:Public head is materialized, and a constrained grant
    // is not stored for the materializing party either.
    let mut g = pod();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(!out.granted, "a constrained grant is not stored: {out:?}");
    assert_eq!(
        cond_grants_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        0,
        "NO auth:Public head — the compound restriction is NOT dropped"
    );

    // Through the real enforcement path: nobody reads n1, so nothing widened.
    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(!reads(&mut store, ALICE), "nothing stored for alice");
    assert!(!reads(&mut store, BOB), "bob (not the recipient) is DENIED — widening closed");
    assert!(
        store.accessible(&Session::default(), Mode::Read).is_empty(),
        "an anonymous session is DENIED — widening closed"
    );

    // End-to-end through query_as: nobody sees the content.
    let sel = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    let bob = Session { agent: Some(BOB), client: None, issuer: None, now: None };
    assert_eq!(store.query_as(&alice, Mode::Read, sel).unwrap().rows.len(), 0);
    assert_eq!(store.query_as(&bob, Mode::Read, sel).unwrap().rows.len(), 0);
    assert_eq!(
        store.query_as(&Session::default(), Mode::Read, sel).unwrap().rows.len(),
        0,
        "anonymous query sees nothing"
    );
}

// 33. A compound-only prohibition (an `odrl:xone` of a single `recipient eq alice`,
//     ZERO atomic constraints) is not re-checked per session, so the CONDITIONAL entry
//     point denies every session; the compound restriction is never dropped.
#[test]
fn compound_only_prohibition_conditional_denies_the_target() {
    let mut store = PodStore::new(pod());
    // Baseline PUBLIC allow so we can observe the deny biting.
    let permit = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/pub> a odrl:Set ; odrl:permission [
            odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ] ."#,
        "turtle",
    )
    .unwrap();
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    install_public_read(&mut store, &permit);
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
    let dout = materialize_prohibition_conditional(&mut g, &prohib, &req);
    assert!(dout.prohibited, "compound-only prohibition materialises a deny for alice: {dout:?}");
    assert_eq!(
        cond_denies_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        1,
        "a compound restriction is not re-checked per session, so it denies everyone"
    );

    // Through the real enforcement path: the deny is unconditional and beats the allow.
    assert!(store.materialize_odrl_prohibition_conditional(&prohib, &req).prohibited);
    assert!(!reads(&mut store, ALICE), "alice (the recipient) is denied — deny-overrides");
    assert!(!reads(&mut store, BOB), "bob is denied too: the deny does not depend on the asker");
    assert!(
        store.accessible(&Session::default(), Mode::Read).is_empty(),
        "an anonymous session is denied"
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
fn lab_assignee_permit() -> sparq_policy::ValidatedPolicy {
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
fn lab_prohibition() -> sparq_policy::ValidatedPolicy {
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
fn lab_carve_out_permit() -> sparq_policy::ValidatedPolicy {
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
fn public_read_permit() -> sparq_policy::ValidatedPolicy {
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

// 42. A collection-valued deny head cannot re-check membership, so freezing it to the
//     evidenced members would let every UNLISTED member escape the prohibition
//     (fail-OPEN). The rule becomes an unconditional deny instead.
#[test]
fn collection_prohibition_stays_one_shot() {
    let prohib = lab_prohibition();
    let req = lab_members(Request::new(odrl("read")).on(N1).by(ALICE));

    let mut g = pod();
    let out = materialize_prohibition_conditional(&mut g, &prohib, &req);
    assert!(out.prohibited, "the member's deny still materializes: {out:?}");
    assert_eq!(cond_denies_for(&g, Some(LAB)), 0, "never the bare collection IRI");
    assert_eq!(
        cond_denies_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        1,
        "a collection cannot be re-checked per session, so the deny is unconditional"
    );

    // End-to-end with deny-overrides: a public allow is in force; each member's own
    // request materializes that member's frozen deny (the one-shot contract).
    let permit = public_read_permit();
    let mut store = PodStore::new(pod());
    install_public_read(&mut store, &permit);
    assert!(reads(&mut store, ALICE), "public allow in force before the deny");
    assert!(store.materialize_odrl_prohibition_conditional(&prohib, &req).prohibited);
    assert!(!reads(&mut store, ALICE), "DENY-OVERRIDES: the member loses access");
    let bob_req = lab_members(Request::new(odrl("read")).on(N1).by(BOB));
    assert!(store.materialize_odrl_prohibition_conditional(&prohib, &bob_req).prohibited);
    assert!(!reads(&mut store, BOB), "bob is denied");
    assert!(!reads(&mut store, CAROL), "the unconditional deny binds a non-member too");
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
    assert!(!out.granted, "carol is allowed, but a constrained grant is not stored: {out:?}");
    assert_eq!(cond_grants_for(&g, None), 0, "a collection carve-out emits NO condition");

    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(!reads(&mut store, CAROL), "nothing stored for carol");
    assert!(!reads(&mut store, BOB), "a member of the EXCLUDED collection must NOT read");
    assert!(!reads(&mut store, ALICE), "no widening to unrelated agents");
}

// 44. THE ZERO-EDGE DENY IS CLOSED: a prohibition naming a collection the request
//     supplied NO `odrl:partOf` edge for used to take the ordinary concrete-head path and
//     persist the bare collection IRI as a deny head — a head no member session can match,
//     so every member escaped the prohibition. Collection identity now comes from the
//     policy's own `a odrl:PartyCollection` declaration, so the rule becomes an
//     unconditional deny regardless of evidence, and never a bare collection head.
#[test]
fn zero_edge_collection_prohibition_emits_no_conditional_deny() {
    let prohib = lab_prohibition();
    // NO `.with_party_membership(…)`: identity comes from the policy declaration alone.
    let req = Request::new(odrl("read")).on(N1).by(CAROL);

    let mut g = pod();
    let out = materialize_prohibition_conditional(&mut g, &prohib, &req);
    assert_eq!(cond_denies_for(&g, None), 1, "one unconditional deny: {out:?}");
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
    install_public_read(&mut store, &permit);
    assert!(store.materialize_odrl_prohibition_conditional(&prohib, &req).prohibited,
        "the deny does not depend on who materialized it");
    assert!(!reads(&mut store, CAROL), "the unconditional deny binds everyone");
    assert!(!reads(&mut store, ALICE), "including every member, evidenced or not");
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
    assert!(!out.granted, "carol is allowed, but a constrained grant is not stored: {out:?}");
    assert_eq!(
        cond_grants_for(&g, None),
        0,
        "NO everyone-except condition is persisted on an unenforceable collection matcher"
    );

    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(!reads(&mut store, CAROL), "nothing stored for carol");
    assert!(!reads(&mut store, BOB), "a member of the EXCLUDED collection must NOT read");
    assert!(!reads(&mut store, ALICE), "nor any other member");
}

// 46. IDENTITY WITHOUT A TYPE TRIPLE: a document that states `<alice> odrl:partOf <lab>`
//     has identified `<lab>` as a collection just as surely as `a odrl:PartyCollection`,
//     and the parser retains both. The prohibition still becomes an unconditional deny
//     even though the REQUEST carries no membership evidence at all.
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
    assert_eq!(
        cond_denies_for(&g, Some("https://sparq.dev/ns/auth#Public")),
        1,
        "the collection rule denies everyone instead"
    );
}

// ===========================================================================
// A conditional grant is re-checked per session, so it may only be emitted when no
// prohibition could apply to some other session. Otherwise the bridge falls back to a
// one-shot grant scoped to the deciding party.
// ===========================================================================

fn cross_session_policy(prohibition_constraint: &str, assignee: &str) -> sparq_policy::ValidatedPolicy {
    parse_policy_str(
        &format!(
            r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/x> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
                      odrl:rightOperand <https://dave.ex/card#me> ] ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <https://pod.ex/notes/n1> {assignee}
    {prohibition_constraint} ] .
"#
        ),
        "turtle",
    )
    .expect("policy parses")
}

#[test]
fn a_prohibition_on_another_party_blocks_the_conditional_grant() {
    // Everyone except dave may read; bob is prohibited. Decided for alice.
    let pol = cross_session_policy("", &format!("; odrl:assignee <{BOB}>"));
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let mut g = pod();
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(!out.granted, "any prohibition blocks a stored grant: {out:?}");
    assert_eq!(cond_grants_for(&g, None), 0, "no head bob could match");

    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(!reads(&mut store, ALICE), "nothing stored for alice");
    assert!(!reads(&mut store, BOB), "bob is prohibited");
    assert!(!reads(&mut store, CAROL), "nor for anyone else");
    assert!(store.accessible(&Session::default(), Mode::Read).is_empty(), "anonymous denied");
}

#[test]
fn a_future_time_prohibition_blocks_the_conditional_grant() {
    // The prohibition opens in 2027: not live now, but any stored grant, conditional or
    // the frozen fallback, would still be read after it opens.
    let c = r#"; odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gteq ;
        odrl:rightOperand "2027-01-01T00:00:00Z"^^xsd:dateTime ]"#;
    let pol = cross_session_policy(c, "");
    let req = Request::new(odrl("read")).on(N1).by(ALICE).at("2026-06-01T00:00:00Z");
    assert!(sparq_policy::evaluate(&pol, &req).allow, "decide grants alice now");
    let mut g = pod();
    let out = materialize_permission_conditional(&mut g, &pol, &req);
    assert!(!out.granted, "nothing stored outlives the window: {out:?}");
    assert_eq!(cond_grants_for(&g, None), 0, "no conditional head");

    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
    let alice_2027 = Session {
        agent: Some(ALICE),
        client: None,
        issuer: None,
        now: Some("2027-06-01T00:00:00Z"),
    };
    assert!(store.accessible(&alice_2027, Mode::Read).is_empty(), "alice denied in 2027");
}

#[test]
fn a_closed_time_prohibition_still_blocks_storage() {
    // The prohibition's window closed in 2025: decide grants alice, but a stored grant
    // is only for policies with no prohibitions at all.
    let c = r#"; odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
        odrl:rightOperand "2025-01-01T00:00:00Z"^^xsd:dateTime ]"#;
    let pol = cross_session_policy(c, "");
    let req = Request::new(odrl("read")).on(N1).by(ALICE).at("2026-06-01T00:00:00Z");
    assert!(sparq_policy::evaluate(&pol, &req).allow, "decide grants alice now");
    let mut store = PodStore::new(pod());
    assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted);
    assert!(!reads(&mut store, ALICE), "nothing stored for alice");
    assert!(!reads(&mut store, CAROL), "nor for anyone else");
}

#[test]
fn an_assignee_and_a_recipient_are_both_required() {
    // Alice is the assignee and Bob the recipient: the evaluator needs both, so no head
    // may admit Bob on the recipient alone.
    let pol = parse_policy_str(
        &format!(
            r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/x> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <{N1}> ; odrl:assignee <{ALICE}> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
                      odrl:rightOperand <{BOB}> ] ] .
"#
        ),
        "turtle",
    )
    .expect("policy parses");
    for by in [ALICE, BOB] {
        let req = Request::new(odrl("read")).on(N1).by(by);
        let mut store = PodStore::new(pod());
        store.materialize_odrl_permission_conditional(&pol, &req);
        assert!(!reads(&mut store, BOB), "bob is not the assignee (decided for {by})");
        assert_eq!(cond_grants_for(&store.graph, None), 0, "no conditional head");
    }
}

#[test]
fn two_recipient_constraints_are_a_conjunction() {
    let pol = parse_policy_str(
        &format!(
            r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/x> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <{N1}> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
                      odrl:rightOperand <{BOB}> ] ,
                    [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
                      odrl:rightOperand <{CAROL}> ] ] .
"#
        ),
        "turtle",
    )
    .expect("policy parses");
    let req = Request::new(odrl("read")).on(N1).by(BOB);
    let mut store = PodStore::new(pod());
    store.materialize_odrl_permission_conditional(&pol, &req);
    assert!(!reads(&mut store, BOB), "bob is not also carol");
    assert!(!reads(&mut store, CAROL), "carol is not also bob");
}

#[test]
fn a_wildcard_party_is_never_granted() {
    // decide() grants the request (bob's prohibition does not name the party), but the
    // party is a principal that many sessions match.
    let pol = cross_session_policy("", &format!("; odrl:assignee <{BOB}>"));
    for party in ["https://sparq.dev/ns/auth#Public", "https://sparq.dev/ns/auth#Authenticated"] {
        let req = Request::new(odrl("read")).on(N1).by(party);
        let mut store = PodStore::new(pod());
        assert!(!store.materialize_odrl_permission(&pol, &req).granted, "{party}");
        assert!(!store.materialize_odrl_permission_conditional(&pol, &req).granted, "{party}");
        assert!(!reads(&mut store, BOB), "bob denied ({party})");
        assert!(store.accessible(&Session::default(), Mode::Read).is_empty(), "{party}");
    }
}

/// Materializing a policy for many requests rebuilds the authorization index once, and
/// leaves the same auth view as materializing each request on its own.
#[test]
fn materializing_for_each_request_rebuilds_the_index_once() {
    let docs: Vec<String> = (0..200).map(|i| format!("https://pod.ex/notes/m{i}")).collect();
    let nq: String = docs
        .iter()
        .map(|d| format!("<{d}#it> <https://ex.dev/ns#title> \"t\" <{d}> .\n"))
        .collect();
    let policy = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/c> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/coll> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .unwrap();
    let requests: Vec<Request> = docs
        .iter()
        .map(|d| Request::new(odrl("read")).on(d.as_str()).by(ALICE).with_asset_membership(d.as_str(), "https://pod.ex/coll"))
        .collect();

    let mut batched = PodStore::new(Graph::load_dataset(&nq, "nquads").unwrap());
    let before = batched.auth_generation();
    let outcomes = batched.materialize_odrl_policy_for_each(&policy, &requests);
    assert_eq!(batched.auth_generation(), before + 1, "one rebuild for {} requests", requests.len());
    assert!(outcomes.iter().all(|o| o.prohibited));

    let mut single = PodStore::new(Graph::load_dataset(&nq, "nquads").unwrap());
    let before = single.auth_generation();
    for r in &requests {
        single.materialize_odrl_policy(&policy, r);
    }
    assert_eq!(single.auth_generation(), before + requests.len() as u64);
    for agent in [ALICE, BOB] {
        let s = Session { agent: Some(agent), client: None, issuer: None, now: None };
        for mode in [Mode::Read, Mode::Write] {
            assert_eq!(batched.accessible(&s, mode), single.accessible(&s, mode), "{agent} {mode:?}");
        }
    }

    // Nothing materialized: no rebuild at all.
    let gen = batched.auth_generation();
    let unrelated = Request::new(odrl("read")).on("https://pod.ex/elsewhere").by(BOB);
    batched.materialize_odrl_policy_for_each(&policy, &[unrelated]);
    assert_eq!(batched.auth_generation(), gen);
}
