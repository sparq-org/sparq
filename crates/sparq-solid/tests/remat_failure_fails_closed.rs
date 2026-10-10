//! A permitted write that changes the access-control rules re-materializes the auth view.
//! The view is rebuilt from the rules the write would leave before the write is committed:
//! rules that cannot be materialized (here an ACL that names a reserved `urn:sparq:` agent,
//! which the loader refuses) refuse the write, which then changes nothing, so the current
//! view still describes the current rules. A write that fails part way changes nothing
//! either, and a controller of one access-control document cannot take the store's whole
//! view down.

use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};

const ADMIN: &str = "https://admin.ex/card#me";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
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
        // Bob controls his own container, and only it.
        "<https://pod.ex/bob/b1#it> <https://ex.dev/ns#title> \"bob's\" <https://pod.ex/bob/b1> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
            <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#accessTo> \
            <https://pod.ex/bob/> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#default> \
            <https://pod.ex/bob/> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#agent> \
            <https://bob.ex/card#me> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Control> <https://pod.ex/bob/.acl> .\n",
    );
    let mut s = PodStore::new(Graph::load_dataset(nq, "nquads").expect("fixture loads"));
    s.materialize_wac().expect("wac materializes");
    s
}

/// Revoke the team's Read and, in the same write, name a reserved agent: rules the view
/// cannot be rebuilt from.
const REVOKE_AND_BREAK: &str = "DELETE DATA { GRAPH <https://pod.ex/.acl> { \
    <https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#agentGroup> <https://pod.ex/groups#team> } } ; \
    INSERT DATA { GRAPH <https://pod.ex/.acl> { \
    <https://pod.ex/.acl#bad> <http://www.w3.org/ns/auth/acl#agent> <urn:sparq:forged> } }";

const REVOKE: &str = "DELETE DATA { GRAPH <https://pod.ex/.acl> { \
    <https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#agentGroup> <https://pod.ex/groups#team> } }";

fn acl_len(store: &PodStore, acl: &str) -> usize {
    store
        .graph
        .named
        .iter()
        .find(|(n, _)| matches!(n, oxrdf::Term::NamedNode(n) if n.as_str() == acl))
        .map_or(0, |(_, g)| g.len())
}

#[test]
fn rules_that_cannot_be_materialized_refuse_the_update() {
    let mut store = store();
    let rules = acl_len(&store, "https://pod.ex/.acl");
    assert!(reads(&store, ALICE), "control: the team reads");
    let err = store
        .update_as(&sess(ADMIN), REVOKE_AND_BREAK)
        .expect_err("the rules cannot be materialized");
    assert!(err.contains("nothing was changed"), "{err}");
    // Nothing changed: not the rules (the revocation and the bad rule are both absent) and
    // not the view, which still describes them.
    assert!(reads(&store, ALICE));
    assert!(reads(&store, ADMIN));
    assert_eq!(acl_len(&store, "https://pod.ex/.acl"), rules);
    // The same revocation, without the bad rule, goes through and takes effect.
    store.update_as(&sess(ADMIN), REVOKE).expect("sound rules");
    assert!(!reads(&store, ALICE), "the revocation took effect");
    assert!(reads(&store, ADMIN));
}

/// Review finding: a rule change that failed part way (here a later operation over its
/// budget) left its earlier operations applied without the view rebuilt from them. A rule
/// change is applied whole or not at all.
#[test]
fn a_rule_change_that_fails_part_way_changes_nothing() {
    let mut store = store();
    let mut budget = sparq_engine::QueryBudget::unlimited();
    budget.max_rows = Some(0);
    let revoke_then_overrun = format!(
        "{REVOKE} ; INSERT {{ GRAPH <{NOTE}> {{ ?s <https://ex.dev/ns#copy> ?o }} }} \
         WHERE {{ GRAPH <{NOTE}> {{ ?s ?p ?o }} }}"
    );
    store
        .update_as_with_budget(&sess(ADMIN), &revoke_then_overrun, &budget)
        .expect_err("the second operation is over budget");
    // The revocation was not applied, and the view still grants what the rules grant.
    assert!(reads(&store, ALICE));
    store.update_as(&sess(ADMIN), REVOKE).expect("sound rules");
    assert!(!reads(&store, ALICE));
}

/// Review finding: a controller of one access-control document could write rules that fail
/// to materialize, and the store dropped its whole view, denying every pod. The write is
/// refused instead; everyone else's access is untouched.
#[test]
fn one_controller_cannot_deny_the_whole_store() {
    let mut store = store();
    let rules = acl_len(&store, "https://pod.ex/bob/.acl");
    let break_own = "INSERT DATA { GRAPH <https://pod.ex/bob/.acl> { \
        <https://pod.ex/bob/.acl#bad> <http://www.w3.org/ns/auth/acl#agent> <urn:sparq:forged> } }";
    store
        .update_as(&sess(BOB), break_own)
        .expect_err("rules that cannot be materialized");
    assert!(reads(&store, ALICE));
    assert!(reads(&store, ADMIN));
    assert!(store
        .accessible(&sess(BOB), Mode::Read)
        .iter()
        .any(|g| g == "https://pod.ex/bob/b1"));
    assert_eq!(acl_len(&store, "https://pod.ex/bob/.acl"), rules);
}
