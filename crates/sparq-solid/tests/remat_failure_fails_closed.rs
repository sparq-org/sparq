//! A permitted write that changes the access-control rules re-materializes the auth view.
//! The view is rebuilt from the rules the write would leave before the write is committed:
//! rules that cannot be materialized refuse the write, which then changes nothing, so the
//! current view still describes the current rules. An ACL naming a reserved `urn:sparq:`
//! agent is rejected whole by the loader instead, so it fails nothing. A write that fails
//! part way changes nothing either, and a controller of one access-control document cannot
//! take the store's whole view down.

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

/// An ACL write naming a reserved agent is refused before anything changes, so the writer
/// keeps the rules (and the Control) it had; the same revocation without it goes through.
#[test]
fn rules_naming_a_reserved_agent_are_refused() {
    let mut store = store();
    assert!(reads(&store, ALICE), "control: the team reads");
    let err = store
        .update_as(&sess(ADMIN), REVOKE_AND_BREAK)
        .expect_err("the write is refused");
    assert!(err.contains("may not state"), "{err}");
    assert!(reads(&store, ALICE), "nothing changed");
    assert!(reads(&store, ADMIN));
    store.update_as(&sess(ADMIN), REVOKE).expect("sound rules");
    assert!(!reads(&store, ALICE), "the revocation took effect");
    assert!(reads(&store, ADMIN));
}

/// Loaded data naming a reserved agent in an ACL is rejected whole by the loader instead: the
/// document governs with no authorizations (fail-closed), and nothing else is affected.
#[test]
fn a_loaded_acl_naming_a_reserved_agent_governs_with_no_grants() {
    let mut store = store();
    let iri = |s: &str| oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s));
    let slot = store
        .graph
        .named
        .iter()
        .position(|(n, _)| *n == iri("https://pod.ex/bob/.acl"))
        .expect("bob's acl");
    let _ = store.graph.named[slot].1.insert_triple(
        iri("https://pod.ex/bob/.acl#bad"),
        iri("http://www.w3.org/ns/auth/acl#agent"),
        iri("urn:sparq:forged"),
    );
    store.materialize_wac().expect("rejected, not fatal");
    assert!(reads(&store, ALICE));
    assert!(reads(&store, ADMIN));
    assert!(store.accessible(&sess("urn:sparq:forged"), Mode::Read).is_empty());
    assert!(
        !store
            .accessible(&sess(BOB), Mode::Read)
            .iter()
            .any(|g| g == "https://pod.ex/bob/b1"),
        "bob's rejected ACL governs his container with no grants"
    );
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
/// to materialize, and the store dropped its whole view, denying every pod. Such a write is
/// refused; everyone else's access, and bob's own, is untouched.
#[test]
fn one_controller_cannot_deny_the_whole_store() {
    let mut store = store();
    let break_own = "INSERT DATA { GRAPH <https://pod.ex/bob/.acl> { \
        <https://pod.ex/bob/.acl#bad> <http://www.w3.org/ns/auth/acl#agent> <urn:sparq:forged> } }";
    store
        .update_as(&sess(BOB), break_own)
        .expect_err("the write is refused");
    assert!(reads(&store, ALICE));
    assert!(reads(&store, ADMIN));
    assert!(store
        .accessible(&sess(BOB), Mode::Read)
        .iter()
        .any(|g| g == "https://pod.ex/bob/b1"));
}

/// Under ACP, a controller cannot make its own access-control resource use a policy of one
/// the store already rejects (which would reject it too, and with it the controller's
/// Control): the write is refused, by update and by replace, and nothing changes.
#[test]
fn an_acr_cannot_take_up_a_rejected_documents_policy() {
    const ACP: &str = "http://www.w3.org/ns/solid/acp#";
    const ACL: &str = "http://www.w3.org/ns/auth/acl#";
    let r = "https://pod.ex/r";
    let acr = format!("{r}.acr");
    let other = "https://pod.ex/other.acr";
    let nq = format!(
        "<{r}#it> <https://ex.dev/ns#k> \"v\" <{r}> .\n\
         <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
         <{acr}#c> <{ACP}apply> <{acr}#p> <{acr}> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Write> <{acr}> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Control> <{acr}> .\n\
         <{acr}#p> <{ACP}anyOf> <{acr}#m> <{acr}> .\n\
         <{acr}#m> <{ACP}agent> <{ADMIN}> <{acr}> .\n\
         <{acr}.acr> <{ACP}accessControl> <{acr}.acr#c> <{acr}.acr> .\n\
         <{acr}.acr#c> <{ACP}apply> <{acr}.acr#p> <{acr}.acr> .\n\
         <{acr}.acr#p> <{ACP}allow> <{ACL}Write> <{acr}.acr> .\n\
         <{acr}.acr#p> <{ACP}anyOf> <{acr}.acr#m> <{acr}.acr> .\n\
         <{acr}.acr#m> <{ACP}agent> <{ADMIN}> <{acr}.acr> .\n\
         <{other}#p> <{ACP}allow> <{ACL}Read> <{other}> .\n\
         <https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\" <{other}> .\n"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    store.materialize_acp().expect("materializes");
    let controls = |store: &PodStore| store.decide(&sess(ADMIN), r, Mode::Control).allow;
    assert!(controls(&store), "control: ADMIN controls r");
    let link = format!("<{acr}#c> <{ACP}apply> <{other}#p>");
    let err = store
        .update_as_acp(
            &sess(ADMIN),
            &format!("INSERT DATA {{ GRAPH <{acr}> {{ {link} }} }}"),
        )
        .expect_err("the update is refused");
    assert!(err.contains("not admitted"), "{err}");
    assert!(controls(&store), "nothing changed");
    let doc = format!(
        "<{acr}> <{ACP}accessControl> <{acr}#c> .\n\
         <{acr}#c> <{ACP}apply> <{acr}#p> .\n\
         {link} .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Read> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Write> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Control> .\n\
         <{acr}#p> <{ACP}anyOf> <{acr}#m> .\n\
         <{acr}#m> <{ACP}agent> <{ADMIN}> .\n"
    );
    let err = store
        .put_acl_acp(&acr, &doc, "ntriples")
        .expect_err("the replace is refused");
    assert!(err.contains("not admitted"), "{err}");
    assert!(controls(&store), "nothing changed");
}
