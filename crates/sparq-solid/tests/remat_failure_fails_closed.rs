//! A permitted write that changes the access-control rules re-materializes the auth view.
//! When that re-materialization fails, the update is already applied, and the previous
//! view would keep granting what the new rules revoke. These tests make it fail (an ACL
//! that names a reserved `urn:sparq:` agent, which the loader refuses) in the same write
//! that revokes a grant, and expect the revoked agent to be denied: the view is dropped,
//! every request fails closed, and a later successful re-materialization restores it.

use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};

const ADMIN: &str = "https://admin.ex/card#me";
const ALICE: &str = "https://alice.ex/card#me";
const NOTE: &str = "https://pod.ex/notes/n1";

fn sess(agent: &str) -> Session<'_> {
    Session {
        agent: Some(agent),
        client: None,
        issuer: None,
        now: None,
    }
}

fn reads(store: &PodStore, agent: &str) -> bool {
    store
        .accessible(&sess(agent), Mode::Read)
        .iter()
        .any(|g| g == NOTE)
}

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
        // ...and controls the root itself, so it may write the root ACL.
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#accessTo> \
            <https://pod.ex/> <https://pod.ex/.acl> .\n",
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

/// Revoke the team's Read and, in the same write, name a reserved agent: the rules change,
/// the view cannot be rebuilt from them.
const REVOKE_AND_BREAK: &str = "DELETE DATA { GRAPH <https://pod.ex/.acl> { \
    <https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#agentGroup> <https://pod.ex/groups#team> } } ; \
    INSERT DATA { GRAPH <https://pod.ex/.acl> { \
    <https://pod.ex/.acl#bad> <http://www.w3.org/ns/auth/acl#agent> <urn:sparq:forged> } }";

#[test]
fn a_failed_rebuild_after_a_revocation_denies() {
    let mut store = store();
    assert!(
        reads(&store, ALICE),
        "control: the team reads before the revocation"
    );
    let err = store
        .update_as(&sess(ADMIN), REVOKE_AND_BREAK)
        .expect_err("the rebuild fails");
    assert!(err.contains("denied until it is re-materialized"), "{err}");
    // The revoked grant does not keep working; nobody's does, until the view is rebuilt.
    assert!(
        !reads(&store, ALICE),
        "a revoked grant outlived a failed rebuild"
    );
    assert!(!reads(&store, ADMIN));
    // Re-materializing from the same broken rules fails again, and stays closed.
    assert!(store.materialize_wac().is_err());
    assert!(!reads(&store, ALICE));
}

#[test]
fn a_rebuild_that_succeeds_restores_access() {
    let mut store = store();
    let err = store
        .update_as(&sess(ADMIN), REVOKE_AND_BREAK)
        .expect_err("the rebuild fails");
    assert!(err.contains("denied until it is re-materialized"), "{err}");
    // The operator replaces the ACL with sound rules: the admin's grant only.
    let acl = concat!(
        "<#owner> a <http://www.w3.org/ns/auth/acl#Authorization> ; ",
        "<http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> ; ",
        "<http://www.w3.org/ns/auth/acl#agent> <https://admin.ex/card#me> ; ",
        "<http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> .",
    );
    let acl = acl.replace("<#owner>", "<https://pod.ex/.acl#owner>");
    store
        .put_acl("https://pod.ex/.acl", &acl, "turtle")
        .expect("sound rules materialize");
    assert!(reads(&store, ADMIN));
    assert!(!reads(&store, ALICE), "the revocation stands");
}
