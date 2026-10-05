//! [SONNET-4.6] issue #55 — a permitted write to a GROUP DOCUMENT auto-re-materializes.
//!
//! `PodStore::update_as` already re-materializes the auth view after a permitted write to
//! an `.acl`/`.acr` document, and after any variable-graph or `CLEAR`/`DROP ALL` update.
//! The remaining hole was the *statically targeted* write to a WAC **group document** — a
//! graph named by some authorization's `acl:agentGroup`. Such a document has no naming
//! convention (`https://pod.ex/groups` looks like any other resource), so the write path
//! could not recognize it, and a `vcard:hasMember` triple removed through `update_as` left
//! the materialized view granting access to an agent who was no longer a member until
//! something else forced a re-materialization.
//!
//! These tests drive the REAL path end to end — the fixture's own `.acl`, the real
//! `update_as` enforcement, and the real read oracle — with NO explicit `materialize_wac`
//! call after the group write. Both directions are covered, because a trigger that only
//! ever grants (or only ever revokes) would be half a fix:
//!
//! - **revoke** (the fail-open-critical direction): removing a membership must drop the
//!   grant on the very next read;
//! - **grant**: adding a membership must be visible on the very next read;
//! - **grants unchanged**: a write to an ordinary content resource changes nobody's
//!   grants (this one pins the observable contract only — it would also hold under a
//!   "re-materialize on every write" implementation, so it is not evidence of scoping);
//! - **still enforced**: the new trigger is not an authorization bypass — a group
//!   document remains an ordinary resource governed by its own ACL, so an agent with
//!   Read only still cannot write it.

use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};

const ADMIN: &str = "https://admin.ex/card#me";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";

/// The group document — deliberately NOT `.acl`/`.acr`-suffixed, so it is recognizable
/// only by being referenced from the ACL via `acl:agentGroup`.
const GROUP_DOC: &str = "https://pod.ex/groups";
const GROUP: &str = "https://pod.ex/groups#team";
const NOTE: &str = "https://pod.ex/notes/n1";

const HAS_MEMBER: &str = "http://www.w3.org/2006/vcard/ns#hasMember";

fn sess(agent: &str) -> Session<'_> {
    Session { agent: Some(agent), client: None, issuer: None, now: None }
}

/// A pod whose root `.acl` gives ADMIN Read+Write+Control over everything and gives the
/// members of `<https://pod.ex/groups#team>` Read — with ALICE the sole initial member.
fn store() -> PodStore {
    let nq = concat!(
        // Content.
        "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/notes/n1> .\n",
        "<https://pod.ex/other#it> <https://ex.dev/ns#title> \"other\" <https://pod.ex/other> .\n",
        // The group document (an ORDINARY resource IRI — no .acl/.acr suffix).
        "<https://pod.ex/groups#team> <http://www.w3.org/2006/vcard/ns#hasMember> \
            <https://alice.ex/card#me> <https://pod.ex/groups> .\n",
        // Root ACL: admin owns the pod.
        "<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
            <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> \
            <https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> \
            <https://admin.ex/card#me> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Control> <https://pod.ex/.acl> .\n",
        // Root ACL: the team group reads the pod.
        "<https://pod.ex/.acl#team> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
            <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#default> \
            <https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#agentGroup> \
            <https://pod.ex/groups#team> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n",
    );
    let mut s = PodStore::new(Graph::load_dataset(nq, "nquads").expect("fixture loads"));
    s.materialize_wac().expect("wac materializes");
    s
}

/// Whether `agent` may read `NOTE`, asked through BOTH the session oracle and the real
/// query path (a stale grant that survives in either one is a fail-open).
fn can_read_note(store: &PodStore, agent: &str) -> bool {
    let s = sess(agent);
    let via_oracle = store
        .accessible(&s, Mode::Read)
        .iter()
        .any(|n| n.as_str() == NOTE);
    let via_query = store
        .ask_as(&s, Mode::Read, &format!("ASK {{ GRAPH <{}> {{ ?s ?p ?o }} }}", NOTE))
        .expect("ask evaluates");
    assert_eq!(via_oracle, via_query, "oracle and query path must agree for <{}>", agent);
    via_oracle
}

fn member_triple(agent: &str) -> String {
    format!(
        "GRAPH <{}> {{ <{}> <{}> <{}> }}",
        GROUP_DOC, GROUP, HAS_MEMBER, agent
    )
}

#[test]
fn group_doc_membership_revoke_rematerializes() {
    let mut s = store();
    assert!(can_read_note(&s, ALICE), "alice starts with a group-derived read grant");

    // ADMIN removes alice's membership through the enforced write path. NOTE: no
    // `materialize_wac()` call follows — the write path must trigger it.
    s.update_as(&sess(ADMIN), &format!("DELETE DATA {{ {} }}", member_triple(ALICE)))
        .expect("admin may write the group document");

    assert!(
        !can_read_note(&s, ALICE),
        "alice's grant must be gone on the very next read: the group-document write did \
         not re-materialize the auth view (stale grant = fail-open)"
    );
}

#[test]
fn group_doc_membership_grant_rematerializes() {
    let mut s = store();
    assert!(!can_read_note(&s, BOB), "bob starts with no grant");

    s.update_as(&sess(ADMIN), &format!("INSERT DATA {{ {} }}", member_triple(BOB)))
        .expect("admin may write the group document");

    assert!(
        can_read_note(&s, BOB),
        "bob's new membership must be visible on the very next read"
    );
}

#[test]
fn ordinary_resource_write_leaves_grants_unchanged() {
    // Negative control: the trigger is scoped to auth-view INPUTS. Writing a resource that
    // no authorization references as a group document changes nobody's grants — so this
    // test would still pass under a "re-materialize on every write" implementation, and it
    // is here to pin the observable contract (grants unchanged), not the trigger count.
    let mut s = store();
    let before: Vec<String> =
        s.accessible(&sess(ALICE), Mode::Read).iter().map(|n| n.as_str().to_owned()).collect();

    s.update_as(
        &sess(ADMIN),
        "INSERT DATA { GRAPH <https://pod.ex/other> { \
            <https://pod.ex/other#it> <https://ex.dev/ns#tag> \"x\" } }",
    )
    .expect("admin may write an ordinary resource");

    let after: Vec<String> =
        s.accessible(&sess(ALICE), Mode::Read).iter().map(|n| n.as_str().to_owned()).collect();
    assert_eq!(before, after, "an ordinary content write must not change any grant");
    assert!(can_read_note(&s, ALICE), "alice keeps her group-derived grant");
}

#[test]
fn group_doc_write_still_requires_write_permission() {
    // The re-materialization trigger must not become an authorization bypass: a group
    // document is an ordinary resource governed by its own ACL, and ALICE holds Read only.
    let mut s = store();
    let err = s
        .update_as(&sess(ALICE), &format!("DELETE DATA {{ {} }}", member_triple(ALICE)))
        .expect_err("alice has Read only on the group document");
    assert!(err.contains("update denied"), "unexpected error: {}", err);
    assert!(
        can_read_note(&s, ALICE),
        "the denied write left the store — and so alice's grant — untouched"
    );
}
