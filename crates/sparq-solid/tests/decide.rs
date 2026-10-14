//! issue #992 Phase-1 (sq-snopa.1/.2/.3) — soundness of the per-resource WAC
//! DECISION layer (`PodStore::decide` / `decide_batch` / `resolve_acl`), exercised through
//! the REAL materialize → AuthIndex::accessible path (no mock bypasses the verdict). This
//! is security-critical authz logic, so the suite pins the soundness cases the issue calls
//! out: allow/deny per mode, agent vs agentGroup vs public vs authenticated, accessTo vs
//! default scope, and the fail-closed contract (ABSENT acl ⇒ deny+NoAcl; present-but-
//! UNLOADED ⇒ deny+Unloaded; transient ⇒ deny+Transient — never allow).

use oxrdf::NamedNode;
use sparq_core::Graph;
use sparq_solid::wac_conformance::AclBuilder;
use sparq_solid::{AclScope, AclStatus, Mode, PodStore, Session};

const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const CAROL: &str = "https://carol.ex/card#me";
const DAVE: &str = "https://dave.ex/card#me";
const TEAM: &str = "https://pod.ex/groups/team#g";

fn session(agent: Option<&str>) -> Session<'_> {
    Session { agent, client: None, issuer: None, now: None }
}

/// Build a materialized WAC store from an ACL corpus.
fn store(acl: AclBuilder) -> PodStore {
    let g = Graph::load_dataset(&acl.into_nquads(), "nquads").expect("loads");
    let mut s = PodStore::new(g);
    s.materialize_wac().expect("materializes");
    s
}

/// `decide(...).allow` — the per-request verdict, the most-checked field.
fn allow(s: &PodStore, agent: Option<&str>, resource: &str, mode: Mode) -> bool {
    s.decide(&session(agent), resource, mode).allow
}

// ─── FR-1: allow / deny per mode (Read/Write/Append/Control) ──────────────────────────

#[test]
fn decide_allow_deny_per_mode() {
    // alice: Read+Write on her own doc; bob: nothing. The doc has its OWN .acl.
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/d.ttl");
    acl.access_to("https://pod.ex/d.ttl", |a| a.agent(ALICE).mode(Mode::Read).mode(Mode::Write));
    let store = store(acl);

    let r = store.decide(&session(Some(ALICE)), "https://pod.ex/d.ttl", Mode::Read);
    assert!(r.allow, "alice has Read");
    assert_eq!(r.status, AclStatus::Resolved);
    assert_eq!(r.scope, Some(AclScope::AccessTo), "own .acl ⇒ accessTo");
    assert_eq!(r.governing_acl, Some(NamedNode::new_unchecked("https://pod.ex/d.ttl.acl")));
    // granted_modes carries the full set in one decision.
    assert!(r.granted_modes.contains(&Mode::Read) && r.granted_modes.contains(&Mode::Write));
    assert!(!r.granted_modes.contains(&Mode::Append) && !r.granted_modes.contains(&Mode::Control));

    assert!(allow(&store, Some(ALICE), "https://pod.ex/d.ttl", Mode::Write));
    // alice was NOT granted Append/Control → authoritative deny (Resolved), not transient.
    let app = store.decide(&session(Some(ALICE)), "https://pod.ex/d.ttl", Mode::Append);
    assert!(!app.allow && app.status == AclStatus::Resolved);
    let ctl = store.decide(&session(Some(ALICE)), "https://pod.ex/d.ttl", Mode::Control);
    assert!(!ctl.allow && ctl.status == AclStatus::Resolved);

    // bob: no grant at all → deny on every mode, but still Resolved (the ACL governs him).
    for m in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
        let d = store.decide(&session(Some(BOB)), "https://pod.ex/d.ttl", m);
        assert!(!d.allow, "bob denied {m:?}");
        assert_eq!(d.status, AclStatus::Resolved);
        assert!(d.granted_modes.is_empty());
    }
}

// ─── FR-1: agent vs agentGroup vs public vs authenticated ─────────────────────────────

#[test]
fn decide_subject_kinds() {
    let mut acl = AclBuilder::new();
    // public Read on /pub/d.ttl; group Read on /team/d.ttl; authenticated-only Read on
    // /auth/d.ttl; alice-only Read on /priv/d.ttl — each its own ACL.
    acl.document("https://pod.ex/pub/d.ttl");
    acl.access_to("https://pod.ex/pub/d.ttl", |a| a.public().mode(Mode::Read));
    acl.document("https://pod.ex/team/d.ttl");
    acl.access_to("https://pod.ex/team/d.ttl", |a| a.agent_group(TEAM, &[CAROL, BOB]).mode(Mode::Read));
    acl.document("https://pod.ex/auth/d.ttl");
    acl.access_to("https://pod.ex/auth/d.ttl", |a| a.authenticated().mode(Mode::Read));
    acl.document("https://pod.ex/priv/d.ttl");
    acl.access_to("https://pod.ex/priv/d.ttl", |a| a.agent(ALICE).mode(Mode::Read));
    let store = store(acl);

    let r = Mode::Read;
    // PUBLIC: anonymous + any agent.
    assert!(allow(&store, None, "https://pod.ex/pub/d.ttl", r));
    assert!(allow(&store, Some(DAVE), "https://pod.ex/pub/d.ttl", r));
    // GROUP: carol + bob yes; dave (non-member) no.
    assert!(allow(&store, Some(CAROL), "https://pod.ex/team/d.ttl", r));
    assert!(allow(&store, Some(BOB), "https://pod.ex/team/d.ttl", r));
    assert!(!allow(&store, Some(DAVE), "https://pod.ex/team/d.ttl", r));
    // AUTHENTICATED: any logged-in agent yes; anonymous no.
    assert!(allow(&store, Some(DAVE), "https://pod.ex/auth/d.ttl", r));
    assert!(!allow(&store, None, "https://pod.ex/auth/d.ttl", r));
    // AGENT: alice yes; bob no.
    assert!(allow(&store, Some(ALICE), "https://pod.ex/priv/d.ttl", r));
    assert!(!allow(&store, Some(BOB), "https://pod.ex/priv/d.ttl", r));
}

// ─── FR-7: accessTo vs default scope; nearest-ancestor discovery ──────────────────────

#[test]
fn decide_scope_access_to_vs_default() {
    // Root .acl grants alice Read by acl:default (members inherit). A deep doc n1 has NO
    // own ACL → inherits with Default scope. A sibling doc WITH its own ACL → AccessTo.
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/notes/n1.ttl");
    acl.document("https://pod.ex/notes/own.ttl");
    acl.default_for("https://pod.ex/", |a| a.agent(ALICE).mode(Mode::Read));
    acl.access_to("https://pod.ex/notes/own.ttl", |a| a.agent(BOB).mode(Mode::Read));
    let store = store(acl);

    // n1: inherited from the root .acl → Default scope.
    let n1 = store.decide(&session(Some(ALICE)), "https://pod.ex/notes/n1.ttl", Mode::Read);
    assert!(n1.allow);
    assert_eq!(n1.scope, Some(AclScope::Default));
    assert_eq!(n1.governing_acl, Some(NamedNode::new_unchecked("https://pod.ex/.acl")));

    // own.ttl: its OWN .acl governs → AccessTo scope, and only bob (alice's default does
    // NOT leak in — nearest-ACL semantics shadow the ancestor grant).
    let eff = store.resolve_acl("https://pod.ex/notes/own.ttl").expect("own acl");
    assert_eq!(eff.scope, AclScope::AccessTo);
    assert_eq!(eff.acl.as_str(), "https://pod.ex/notes/own.ttl.acl");
    assert!(allow(&store, Some(BOB), "https://pod.ex/notes/own.ttl", Mode::Read));
    let alice_on_own = store.decide(&session(Some(ALICE)), "https://pod.ex/notes/own.ttl", Mode::Read);
    assert!(!alice_on_own.allow, "nearest-ACL shadows the root acl:default grant");
    assert_eq!(alice_on_own.status, AclStatus::Resolved);

    // The /notes/ container is a MEMBER of / , so the root's acl:default <pod.ex/> DOES
    // grant it (inherited) — Default scope, governed by the root .acl. The ROOT container
    // itself is NOT a member of itself, so acl:default does not grant accessTo on it →
    // deny, while its own .acl is still surfaced (AccessTo discovery scope).
    let notes = store.decide(&session(Some(ALICE)), "https://pod.ex/notes/", Mode::Read);
    assert!(notes.allow, "/notes/ is a member of / → inherited grant");
    assert_eq!(notes.scope, Some(AclScope::Default));
    let root = store.decide(&session(Some(ALICE)), "https://pod.ex/", Mode::Read);
    assert!(!root.allow, "acl:default does not grant accessTo on the container it targets");
    assert_eq!(root.scope, Some(AclScope::AccessTo), "root's own .acl governs it directly");
}

#[test]
fn resolve_acl_nearest_ancestor_one_call() {
    // doc → /a/b/ → /a/ → / : only /a/ has an ACL → /a/.acl governs by default.
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/a/b/doc.ttl");
    acl.default_for("https://pod.ex/a/", |a| a.public().mode(Mode::Read));
    let store = store(acl);
    let eff = store.resolve_acl("https://pod.ex/a/b/doc.ttl").expect("inherited");
    assert_eq!(eff.acl.as_str(), "https://pod.ex/a/.acl");
    assert_eq!(eff.scope, AclScope::Default);
}

// ─── FR-5: effective-ACL provenance surface — Link: rel="acl" + scope predicate ───────

#[test]
fn acl_link_header_surfaces_governing_acl_over_real_decide() {
    // The FR-5 discoverability surface, exercised through the REAL public `decide` path
    // (materialize → AuthIndex::accessible → decision), not the internal helper.
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/notes/n1.ttl");
    acl.document("https://pod.ex/notes/own.ttl");
    acl.default_for("https://pod.ex/", |a| a.agent(ALICE).mode(Mode::Read));
    acl.access_to("https://pod.ex/notes/own.ttl", |a| a.agent(BOB).mode(Mode::Read));
    let store = store(acl);

    // Inherited (Default scope) → the ancestor .acl is advertised as the rel=acl Link.
    let n1 = store.decide(&session(Some(ALICE)), "https://pod.ex/notes/n1.ttl", Mode::Read);
    assert_eq!(
        n1.acl_link_header().as_deref(),
        Some(r#"<https://pod.ex/.acl>; rel="acl""#)
    );
    assert_eq!(n1.scope.map(AclScope::as_acl_predicate), Some("http://www.w3.org/ns/auth/acl#default"));

    // Own ACL (AccessTo scope) → the resource's OWN .acl is advertised.
    let own = store.decide(&session(Some(BOB)), "https://pod.ex/notes/own.ttl", Mode::Read);
    assert!(own.allow);
    assert_eq!(
        own.acl_link_header().as_deref(),
        Some(r#"<https://pod.ex/notes/own.ttl.acl>; rel="acl""#)
    );
    assert_eq!(own.scope.map(AclScope::as_acl_predicate), Some("http://www.w3.org/ns/auth/acl#accessTo"));

    // The Link surface is independent of the verdict — an authoritative DENY still tells the
    // client WHERE the governing ACL is (safe: it surfaces provenance, never the grant).
    let denied = store.decide(&session(Some(BOB)), "https://pod.ex/notes/n1.ttl", Mode::Read);
    assert!(!denied.allow, "bob has no grant on n1");
    assert_eq!(denied.status, AclStatus::Resolved);
    assert_eq!(
        denied.acl_link_header().as_deref(),
        Some(r#"<https://pod.ex/.acl>; rel="acl""#)
    );
}

#[test]
fn acl_link_header_none_is_fail_closed_over_real_decide() {
    // Fail-closed: an un-protected resource (NoAcl) and a malformed IRI (Transient) have no
    // governing ACL, so there is nothing to advertise — `acl_link_header()` is `None`.
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/open.ttl"); // a doc with zero authorizations anywhere
    let store = store(acl);

    let no_acl = store.decide(&session(Some(ALICE)), "https://pod.ex/open.ttl", Mode::Read);
    assert_eq!(no_acl.status, AclStatus::NoAcl);
    assert!(no_acl.acl_link_header().is_none(), "no ACL ⇒ nothing to advertise");

    let transient = store.decide(&session(Some(ALICE)), "not a valid iri", Mode::Read);
    assert_eq!(transient.status, AclStatus::Transient);
    assert!(transient.acl_link_header().is_none());
}

// ─── FR-6: the fail-closed load/error contract (the security core) ────────────────────

#[test]
fn fail_closed_absent_acl_is_denied_noacl() {
    // A resource with NO ACL anywhere in its chain: deny + NoAcl (definitive 403), and
    // CRUCIALLY not an allow. The auth view IS materialized — the absence is structural.
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/open.ttl"); // a doc, but zero authorizations anywhere
    let store = store(acl);
    let d = store.decide(&session(Some(ALICE)), "https://pod.ex/open.ttl", Mode::Read);
    assert!(!d.allow, "ABSENT acl ⇒ DENIED, never allowed");
    assert_eq!(d.status, AclStatus::NoAcl);
    assert!(!d.status.is_retryable(), "absent acl is a definitive deny");
    assert!(d.governing_acl.is_none());
    assert!(store.resolve_acl("https://pod.ex/open.ttl").is_none());
}

#[test]
fn fail_closed_present_but_unloaded_is_denied_unloaded() {
    // The ACL document is PRESENT in the dataset, but materialize_* was NEVER run, so no
    // verdict can be computed → deny + Unloaded (retryable 503), never allow.
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/d.ttl");
    acl.access_to("https://pod.ex/d.ttl", |a| a.agent(ALICE).mode(Mode::Read));
    let g = Graph::load_dataset(&acl.into_nquads(), "nquads").expect("loads");
    let store = PodStore::new(g); // NOTE: no materialize_wac()
    let d = store.decide(&session(Some(ALICE)), "https://pod.ex/d.ttl", Mode::Read);
    assert!(!d.allow, "present-but-UNLOADED ⇒ DENIED, never allowed");
    assert_eq!(d.status, AclStatus::Unloaded);
    assert!(d.status.is_retryable(), "unloaded is a retryable 503, not a permission denial");
    // The governing ACL is still discovered (for the Link: rel=acl surface / a 503 body).
    assert_eq!(d.governing_acl, Some(NamedNode::new_unchecked("https://pod.ex/d.ttl.acl")));
}

#[test]
fn fail_closed_transient_on_malformed_resource_iri() {
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/d.ttl");
    acl.access_to("https://pod.ex/d.ttl", |a| a.public().mode(Mode::Read));
    let store = store(acl);
    let d = store.decide(&session(Some(ALICE)), "this is not an iri", Mode::Read);
    assert!(!d.allow, "malformed resource IRI ⇒ DENIED, never allowed");
    assert_eq!(d.status, AclStatus::Transient);
    assert!(d.status.is_retryable());
    assert!(d.governing_acl.is_none());
}

#[test]
fn fail_closed_reserved_encoding_session_value() {
    // A session whose "WebID" is itself in the reserved minted-principal space must never
    // match a grant (it could otherwise impersonate a minted pair). decide must fail closed
    // even when the ACL/material view exists — the reserved value invalidates the WHOLE
    // session (accessible returns empty), so even Public is not reachable for it.
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/d.ttl");
    acl.access_to("https://pod.ex/d.ttl", |a| a.public().mode(Mode::Read));
    let store = store(acl);
    let forged = Session {
        agent: Some("urn:sparq:pair?agent=x&client=y"),
        client: None,
        issuer: None,
        now: None,
    };
    let d = store.decide(&forged, "https://pod.ex/d.ttl", Mode::Read);
    assert!(!d.allow, "reserved-encoding session value ⇒ fail-closed deny");
}

// ─── FR-1: decide_batch is a per-element independent, fail-closed decision ────────────

#[test]
fn decide_batch_independent_decisions() {
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/pub.ttl");
    acl.access_to("https://pod.ex/pub.ttl", |a| a.public().mode(Mode::Read));
    acl.document("https://pod.ex/priv.ttl");
    acl.access_to("https://pod.ex/priv.ttl", |a| a.agent(ALICE).mode(Mode::Read));
    let store = store(acl);

    let bob = session(Some(BOB));
    let reqs: &[(&str, Mode)] = &[
        ("https://pod.ex/pub.ttl", Mode::Read),     // public → allow
        ("https://pod.ex/priv.ttl", Mode::Read),    // alice-only, bob → deny (Resolved)
        ("https://pod.ex/missing.ttl", Mode::Read), // no ACL → deny (NoAcl)
        ("https://pod.ex/pub.ttl", Mode::Write),    // public has only Read → deny
    ];
    let out = store.decide_batch(&bob, reqs);
    assert_eq!(out.len(), 4, "result is parallel to input");
    assert!(out[0].allow && out[0].status == AclStatus::Resolved);
    assert!(!out[1].allow && out[1].status == AclStatus::Resolved);
    assert!(!out[2].allow && out[2].status == AclStatus::NoAcl);
    assert!(!out[3].allow && out[3].status == AclStatus::Resolved);

    // The batch result must equal calling decide() one-by-one (no cross-talk).
    for (i, (res, mode)) in reqs.iter().enumerate() {
        assert_eq!(store.decide(&bob, res, *mode), out[i], "batch == singletons at {i}");
    }
}

// ─── Equivalence: decide's verdict matches the accessible() oracle (no widening) ──────

#[test]
fn decide_verdict_matches_accessible_oracle() {
    // decide() must never grant more than accessible()/query_as would. Cross-check the
    // verdict against the graph-filtering oracle for every (session, mode) on a resource
    // the oracle CAN see.
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/notes/n1.ttl");
    acl.default_for("https://pod.ex/", |a| a.agent(ALICE).mode(Mode::Read).mode(Mode::Write));
    let s = store(acl);

    let resource = "https://pod.ex/notes/n1.ttl";
    let res_node = NamedNode::new_unchecked(resource);
    for agent in [Some(ALICE), Some(BOB), None] {
        for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
            let sess = session(agent);
            let oracle = s.accessible(&sess, mode).contains(&res_node);
            let decided = s.decide(&sess, resource, mode).allow;
            assert_eq!(decided, oracle, "decide != accessible for {agent:?}/{mode:?}");
        }
    }
}

// ─── sq-j8qtt (issue #1570): persistent AclIndex — INVALIDATION must be airtight ──────
//
// The structural ACL index that `decide`/`decide_batch`/`resolve_acl` walk is now built
// ONCE per generation and reused (not rebuilt per call). A stale index after an ACL change
// is a WRONG authorization, so these pin that a rule change is reflected by the very next
// decision on the SAME store — in BOTH directions (a newly-granting ACL, and the
// fail-open-critical case of a DELETED granting ACL) — through the real put_acl/delete_acl
// → reindex seam, and that the lazy build is race-free under a shared `&self`.

/// A materialized WAC store with content but NO access-control document anywhere.
fn empty_store() -> PodStore {
    let nquads =
        "<https://pod.ex/notes/n1.ttl#it> <https://ex.dev/ns#title> \"hi\" <https://pod.ex/notes/n1.ttl> .\n";
    let g = Graph::load_dataset(nquads, "nquads").expect("loads");
    let mut s = PodStore::new(g);
    s.materialize_wac().expect("materializes");
    s
}

#[test]
fn decide_reflects_put_acl_without_manual_reindex() {
    // Warm the persistent index on a pod with NO ACL (a NoAcl deny), then authoritatively
    // PUT a root .acl granting alice Read. The very next decide() on the SAME store must see
    // the new grant — a stale index would still answer NoAcl (a wrong, fail-closed verdict).
    let mut store = empty_store();
    let alice = session(Some(ALICE));
    let resource = "https://pod.ex/notes/n1.ttl";

    let before = store.decide(&alice, resource, Mode::Read);
    assert!(!before.allow && before.status == AclStatus::NoAcl, "no ACL yet ⇒ NoAcl deny");
    assert!(store.resolve_acl(resource).is_none(), "warm the index: no control document yet");

    let acl = format!(
        "<https://pod.ex/.acl#o> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> .\n\
         <https://pod.ex/.acl#o> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> .\n\
         <https://pod.ex/.acl#o> <http://www.w3.org/ns/auth/acl#agent> <{}> .\n\
         <https://pod.ex/.acl#o> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> .\n",
        ALICE
    );
    store.put_acl("https://pod.ex/.acl", &acl, "ntriples").expect("put_acl");

    let after = store.decide(&alice, resource, Mode::Read);
    assert!(after.allow, "put_acl grant visible to the next decide (index was invalidated)");
    assert_eq!(after.status, AclStatus::Resolved);
    assert_eq!(after.scope, Some(AclScope::Default));
    assert_eq!(
        store.resolve_acl(resource).map(|e| e.acl),
        Some(NamedNode::new_unchecked("https://pod.ex/.acl")),
        "resolve_acl also sees the new control document"
    );
}

#[test]
fn decide_reflects_delete_acl_no_stale_grant() {
    // The fail-OPEN-critical direction: an ACL that GRANTED access is DELETED. Warm the
    // index (alice granted Read via the resource's OWN .acl), delete that .acl, and the next
    // decision on the same store must STOP granting — a stale structural index would keep
    // discovering the deleted control document.
    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/d.ttl");
    acl.access_to("https://pod.ex/d.ttl", |a| a.agent(ALICE).mode(Mode::Read));
    let mut store = store(acl);
    let alice = session(Some(ALICE));
    let resource = "https://pod.ex/d.ttl";

    let before = store.decide(&alice, resource, Mode::Read);
    assert!(before.allow && before.status == AclStatus::Resolved, "granted via own .acl");
    assert!(store.resolve_acl(resource).is_some(), "warm the index: control document present");

    store.delete_acl("https://pod.ex/d.ttl.acl").expect("delete_acl");

    let after = store.decide(&alice, resource, Mode::Read);
    assert!(!after.allow, "deleted ACL must not keep granting (index was invalidated)");
    assert_eq!(after.status, AclStatus::NoAcl, "no control document remains ⇒ NoAcl");
    assert!(store.resolve_acl(resource).is_none(), "resolve_acl no longer finds the deleted ACL");
}

#[test]
fn concurrent_decide_shares_one_persistent_index() {
    // `decide`/`resolve_acl` are `&self`; a per-request LDP server shares `&PodStore` across
    // threads. The persistent index must build ONCE under contention and every thread must
    // see the correct verdict — asserts PodStore stays Send+Sync and the lazy build is
    // race-free.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PodStore>();

    let mut acl = AclBuilder::new();
    acl.document("https://pod.ex/d.ttl");
    acl.access_to("https://pod.ex/d.ttl", |a| a.agent(ALICE).mode(Mode::Read));
    let shared = std::sync::Arc::new(store(acl));

    let mut handles = Vec::new();
    for _ in 0..8 {
        let s = std::sync::Arc::clone(&shared);
        handles.push(std::thread::spawn(move || {
            for _ in 0..200 {
                assert!(s.decide(&session(Some(ALICE)), "https://pod.ex/d.ttl", Mode::Read).allow);
                assert!(!s.decide(&session(Some(BOB)), "https://pod.ex/d.ttl", Mode::Read).allow);
                assert!(s.resolve_acl("https://pod.ex/d.ttl").is_some());
            }
        }));
    }
    for h in handles {
        h.join().expect("thread ok");
    }
}
