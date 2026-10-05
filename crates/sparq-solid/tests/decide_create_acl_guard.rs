//! [OPUS-5] sq-gg0qq.5 (issue #2571) — the POST-`Slug` access-control-document
//! privilege-escalation guard, asserted against the REAL `materialize_wac` →
//! `AuthIndex::accessible` path (no mock short-circuits the verdict).
//!
//! ## The bug class
//!
//! Minting a child of a container requires only `acl:Append`; an access-control document
//! is governed by `acl:Control`. A server that authorizes a create by asking only "does
//! this principal hold Append on the container?" therefore lets an Append-only principal
//! POST `Slug: secret.acl` and walk away having authored `<container>/secret.acl` — the
//! document the ACL resolver afterwards consults as `<container>/secret`'s own governing
//! ACL. The mode check is correct and still misses it, because the escalation is carried
//! by the NAME.
//!
//! `PodStore::decide_create` is the decision-engine-side chokepoint: any resource server
//! that delegates its create authorization here inherits the refusal instead of having to
//! re-derive it at its own mint site. These tests are the acceptance bar for that — the
//! escalation attempt must fail in every spelling, and it must fail for a CONTROLLER too.

use sparq_core::Graph;
use sparq_solid::{is_control_document_name, AclStatus, Mode, PodStore, Session};

const ALICE: &str = "https://alice.ex/card#me"; // holds Control (the container's owner)
const BOB: &str = "https://bob.ex/card#me"; // holds Append only (the attacker)
const CONTAINER: &str = "https://pod.ex/notes/";

/// A pod whose `/notes/` container ACL grants alice full Control and bob only Append, by
/// `acl:default` — the exact shape a "public drop-box" / shared-inbox container has, and
/// the shape the escalation is aimed at.
fn store() -> PodStore {
    let nquads = format!(
        r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#k> "v" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <{CONTAINER}> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#owner> <http://www.w3.org/ns/auth/acl#accessTo> <{CONTAINER}> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <{ALICE}> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Append> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Control> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#drop> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#drop> <http://www.w3.org/ns/auth/acl#accessTo> <{CONTAINER}> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#drop> <http://www.w3.org/ns/auth/acl#agent> <{BOB}> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#drop> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Append> <https://pod.ex/notes/.acl> .
"#
    );
    let mut s = PodStore::new(Graph::load_dataset(&nquads, "nquads").expect("loads"));
    s.materialize_wac().expect("materializes");
    s
}

fn session(agent: &str) -> Session<'_> {
    Session { agent: Some(agent), client: None, issuer: None, now: None }
}

/// The fixture is only meaningful if bob really DOES hold Append on the container — i.e.
/// if the refusals below are the guard talking, not an incidental deny.
#[test]
fn fixture_actually_grants_the_attacker_append_on_the_container() {
    let s = store();
    let d = s.decide(&session(BOB), CONTAINER, Mode::Append);
    assert!(d.allow, "the escalation premise: bob may append to the container");
    assert_eq!(d.status, AclStatus::Resolved);

    // …and the benign create therefore succeeds, so a refusal below is about the NAME.
    let ok = s.decide_create(&session(BOB), CONTAINER, "note1", Mode::Append);
    assert!(ok.allow, "a benign child name must still be creatable");
    assert_eq!(ok.status, AclStatus::Resolved);
}

/// The headline guard: an Append-only principal cannot mint an access-control document,
/// in ANY spelling — case variants, the container-child trailing slash, percent-encoded
/// dots/letters, a smuggled encoded separator, and double encoding.
#[test]
fn append_only_principal_cannot_mint_an_acl_document() {
    let s = store();
    let attempts = [
        "secret.acl",
        "secret.ACL",
        "secret.Acl",
        "secret.acl/",
        ".acl",
        "secret.acr",
        "secret.ACR",
        ".acr",
        "secret%2Eacl",
        "secret%2eacl",
        "secret.ac%6C",
        "secret.acl%2F",
        "secret%2Fnested.acl",
        "secret%252Eacl",
    ];
    for name in attempts {
        let d = s.decide_create(&session(BOB), CONTAINER, name, Mode::Append);
        assert!(
            !d.allow,
            "an Append-only principal minted the control document `{}`",
            name
        );
        assert!(
            d.granted_modes.is_empty(),
            "`{}` must report no granted mode for this operation",
            name
        );
        assert_eq!(
            d.status,
            AclStatus::Resolved,
            "`{}` must be a DEFINITIVE (403) refusal",
            name
        );
        assert!(
            !d.status.is_retryable(),
            "`{}` must not invite a retry",
            name
        );
    }
}

/// Asking for `Write` instead of `Append` must not route around the guard — the refusal
/// is a property of the name, not of the mode the caller happens to request.
#[test]
fn requesting_a_different_mode_does_not_bypass_the_guard() {
    let s = store();
    for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
        let d = s.decide_create(&session(BOB), CONTAINER, "secret.acl", mode);
        assert!(!d.allow, "mode {:?} bypassed the control-document guard", mode);
    }
}

/// A CONTROLLER is refused too. The legitimate way to author an access-control document
/// is a `Control`-gated write of the governed resource's own ACL — never a child mint —
/// so the create path stays uniformly closed and the guard has no exception to aim at.
#[test]
fn even_a_controller_cannot_mint_an_acl_document_through_create() {
    let s = store();
    // Premise: alice really does hold Control here.
    assert!(
        s.decide(&session(ALICE), CONTAINER, Mode::Control).allow,
        "the fixture must grant alice Control for this test to mean anything"
    );

    let d = s.decide_create(&session(ALICE), CONTAINER, "secret.acl", Mode::Write);
    assert!(!d.allow, "the create path is closed for everyone, controllers included");
    assert_eq!(d.status, AclStatus::Resolved);

    // The legitimate route is still open: WAC turns Control on `<R>` into Read+Write on
    // `<R>`'s OWN access-control document, so alice edits `/notes/.acl` directly.
    let legit = s.decide(&session(ALICE), "https://pod.ex/notes/.acl", Mode::Write);
    assert!(
        legit.allow,
        "a controller must still be able to write the container's own ACL"
    );
    // …and bob, holding only Append, cannot.
    assert!(!s.decide(&session(BOB), "https://pod.ex/notes/.acl", Mode::Write).allow);
}

/// An attacker cannot SHADOW an existing ACL either: the refusal does not depend on
/// whether the target control document already exists.
#[test]
fn cannot_shadow_an_existing_acl_document() {
    let s = store();
    // `/notes/.acl` exists in the fixture; `/notes/n1.acl` does not.
    for name in [".acl", "n1.acl"] {
        let d = s.decide_create(&session(BOB), CONTAINER, name, Mode::Append);
        assert!(!d.allow, "`{}` must be refused whether or not it exists", name);
    }
}

/// Path traversal and other unusable child names are refused rather than resolved against
/// some other container — an ambiguous minted name is an ambiguous governing ACL.
#[test]
fn unusable_child_names_are_refused() {
    let s = store();
    for name in ["", "..", ".", "/", "a/b", "a%2Fb", "%2E%2E"] {
        let d = s.decide_create(&session(BOB), CONTAINER, name, Mode::Append);
        assert!(!d.allow, "unusable child name `{}` was accepted", name);
        assert_eq!(d.status, AclStatus::Resolved);
    }
    // A non-container target is refused too (a create needs a container to create IN).
    let d = s.decide_create(&session(BOB), "https://pod.ex/notes/n1", "child", Mode::Append);
    assert!(!d.allow);
}

/// Fail-closed on decision-engine error: with the auth view NEVER materialized, a benign
/// create is a RETRYABLE deny (503) and the escalation attempt is still a DEFINITIVE one.
/// Neither is ever an allow.
#[test]
fn unmaterialized_view_fails_closed_for_both_benign_and_hostile_names() {
    let nquads = format!(
        r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#k> "v" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/.acl#drop> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#drop> <http://www.w3.org/ns/auth/acl#accessTo> <{CONTAINER}> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#drop> <http://www.w3.org/ns/auth/acl#agent> <{BOB}> <https://pod.ex/notes/.acl> .
<https://pod.ex/notes/.acl#drop> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Append> <https://pod.ex/notes/.acl> .
"#
    );
    // NOTE: `materialize_wac` is deliberately NOT called.
    let s = PodStore::new(Graph::load_dataset(&nquads, "nquads").expect("loads"));

    let benign = s.decide_create(&session(BOB), CONTAINER, "note1", Mode::Append);
    assert!(!benign.allow, "fail-closed with no auth view");
    assert_eq!(benign.status, AclStatus::Unloaded);
    assert!(benign.status.is_retryable(), "operational, so retryable (503)");

    let hostile = s.decide_create(&session(BOB), CONTAINER, "secret.acl", Mode::Append);
    assert!(!hostile.allow);
    assert_eq!(
        hostile.status,
        AclStatus::Resolved,
        "the name refusal is permanent and must not be reported as retryable"
    );
    assert!(!hostile.status.is_retryable());
}

/// An anonymous principal gets nothing, benign name or not.
#[test]
fn anonymous_cannot_create_anything() {
    let s = store();
    let anon = Session::default();
    assert!(!s.decide_create(&anon, CONTAINER, "note1", Mode::Append).allow);
    assert!(!s.decide_create(&anon, CONTAINER, "secret.acl", Mode::Append).allow);
}

/// The public predicate a resource server reuses at its own mint chokepoint agrees with
/// the decision it gates — the property that keeps the two from drifting apart.
#[test]
fn the_public_predicate_agrees_with_the_decision() {
    let s = store();
    for name in ["secret.acl", "secret.ACL", ".acr", "note1", "note.ttl", "aclremap"] {
        let refused_by_predicate = is_control_document_name(name);
        let allowed = s.decide_create(&session(BOB), CONTAINER, name, Mode::Append).allow;
        assert_eq!(
            refused_by_predicate, !allowed,
            "`{}`: the predicate and the decision disagree",
            name
        );
    }
}
