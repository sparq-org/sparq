//! The trusted ACP context (creator/owner provenance and verified credentials) a store was
//! materialized with stays in force across the rebuilds the store makes itself: after an
//! authoritative `.acr` replace or delete, and after an authorized update, the grants and
//! denies that context decides are unchanged. A resource that goes keeps none of it unless
//! its own `.acr` is still stored: one created again under the same IRI has no owner, and
//! its creator is whoever created it again.

use sparq_core::Graph;
use sparq_solid::{AccessProvenance, Mode, PodStore, Session, VerifiedCredentials};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const D0: &str = "https://pod.ex/docs/d0";
const D1: &str = "https://pod.ex/docs/d1"; // no creator or owner
const ALICE: &str = "https://alice.ex/card#me"; // created d0
const ERIN: &str = "https://erin.ex/card#me"; // owns d0
const CAROL: &str = "https://carol.ex/card#me"; // holds the flagged credential
const DAVE: &str = "https://dave.ex/card#me"; // no context at all
const ADMIN: &str = "https://admin.ex/card#me";
const FLAGGED: &str = "https://issuer.ex/req#Flagged";

/// `/docs/` policies: everyone reads, the creator and ADMIN write, the creator is denied
/// Read, and the owner and holders of the flagged credential are denied Read. `/other.acr` is unrelated; ADMIN holds Write
/// on it through `/other.acr.acr`.
fn store() -> PodStore {
    let docs = "https://pod.ex/docs/.acr";
    let mut nq = format!(
        "<{D0}#it> <https://ex.dev/ns#k> \"v\" <{D0}> .\n\
         <https://pod.ex/other#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/other> .\n\
         <{D1}#it> <https://ex.dev/ns#k> \"v\" <{D1}> .\n"
    );
    let quads = [
        (docs, docs, "memberAccessControl", format!("{docs}#c")),
        (
            docs,
            &format!("{docs}#c"),
            "apply",
            format!("{docs}#everyone"),
        ),
        (
            docs,
            &format!("{docs}#c"),
            "apply",
            format!("{docs}#creator"),
        ),
        (
            docs,
            &format!("{docs}#c"),
            "apply",
            format!("{docs}#denyOwner"),
        ),
        (
            docs,
            &format!("{docs}#c"),
            "apply",
            format!("{docs}#denyFlagged"),
        ),
        (
            docs,
            &format!("{docs}#everyone"),
            "allow",
            format!("{ACL}Read"),
        ),
        (
            docs,
            &format!("{docs}#everyone"),
            "anyOf",
            format!("{docs}#public"),
        ),
        (
            docs,
            &format!("{docs}#public"),
            "agent",
            format!("{ACP}PublicAgent"),
        ),
        (
            docs,
            &format!("{docs}#creator"),
            "allow",
            format!("{ACL}Write"),
        ),
        (
            docs,
            &format!("{docs}#creator"),
            "allOf",
            format!("{docs}#mc"),
        ),
        (
            docs,
            &format!("{docs}#mc"),
            "agent",
            format!("{ACP}CreatorAgent"),
        ),
        (
            docs,
            &format!("{docs}#c"),
            "apply",
            format!("{docs}#denyCreator"),
        ),
        (
            docs,
            &format!("{docs}#denyCreator"),
            "deny",
            format!("{ACL}Read"),
        ),
        (
            docs,
            &format!("{docs}#denyCreator"),
            "allOf",
            format!("{docs}#mc"),
        ),
        (
            docs,
            &format!("{docs}#denyOwner"),
            "deny",
            format!("{ACL}Read"),
        ),
        (
            docs,
            &format!("{docs}#denyOwner"),
            "allOf",
            format!("{docs}#mo"),
        ),
        (
            docs,
            &format!("{docs}#mo"),
            "agent",
            format!("{ACP}OwnerAgent"),
        ),
        (
            docs,
            &format!("{docs}#denyFlagged"),
            "deny",
            format!("{ACL}Read"),
        ),
        (
            docs,
            &format!("{docs}#denyFlagged"),
            "allOf",
            format!("{docs}#mf"),
        ),
        (docs, &format!("{docs}#mf"), "vc", FLAGGED.to_owned()),
        (
            docs,
            &format!("{docs}#c"),
            "apply",
            format!("{docs}#editor"),
        ),
        (
            docs,
            &format!("{docs}#editor"),
            "allow",
            format!("{ACL}Write"),
        ),
        (
            docs,
            &format!("{docs}#editor"),
            "anyOf",
            format!("{docs}#me"),
        ),
        (docs, &format!("{docs}#me"), "agent", ADMIN.to_owned()),
        (
            "https://pod.ex/other.acr.acr",
            "https://pod.ex/other.acr.acr",
            "accessControl",
            "https://pod.ex/other.acr.acr#c".to_owned(),
        ),
        (
            "https://pod.ex/other.acr.acr",
            "https://pod.ex/other.acr.acr#c",
            "apply",
            "https://pod.ex/other.acr.acr#p".to_owned(),
        ),
        (
            "https://pod.ex/other.acr.acr",
            "https://pod.ex/other.acr.acr#p",
            "allow",
            format!("{ACL}Write"),
        ),
        (
            "https://pod.ex/other.acr.acr",
            "https://pod.ex/other.acr.acr#p",
            "anyOf",
            "https://pod.ex/other.acr.acr#m".to_owned(),
        ),
        (
            "https://pod.ex/other.acr.acr",
            "https://pod.ex/other.acr.acr#m",
            "agent",
            ADMIN.to_owned(),
        ),
    ];
    for (g, s, p, o) in quads {
        nq += &format!("<{s}> <{ACP}{p}> <{o}> <{g}> .\n");
    }
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("fixture loads"));
    let mut prov = AccessProvenance::new();
    prov.set_creator(D0, ALICE);
    prov.set_owner(D0, ERIN);
    let mut creds = VerifiedCredentials::new();
    creds.hold(CAROL, FLAGGED);
    store
        .materialize_acp_with_credentials(&prov, &creds)
        .expect("acp materializes");
    store
}

fn holds(store: &PodStore, agent: &str, mode: Mode) -> bool {
    let s = Session {
        agent: Some(agent),
        client: None,
        issuer: None,
        now: None,
    };
    store.decide(&s, D0, mode).allow
}

fn assert_context_in_force(store: &PodStore, after: &str) {
    assert!(
        !holds(store, ERIN, Mode::Read),
        "{after}: the owner deny holds"
    );
    assert!(
        !holds(store, CAROL, Mode::Read),
        "{after}: the credential deny holds"
    );
    assert!(
        !holds(store, ALICE, Mode::Read),
        "{after}: the creator deny holds"
    );
    assert!(holds(store, DAVE, Mode::Read), "{after}: everyone reads");
    assert!(
        holds(store, ALICE, Mode::Write),
        "{after}: the creator's grant holds"
    );
    assert!(
        !holds(store, DAVE, Mode::Write),
        "{after}: only the creator writes"
    );
}

const OTHER_ACR: &str =
    "<https://pod.ex/other.acr> <http://www.w3.org/ns/solid/acp#accessControl> \
    <https://pod.ex/other.acr#c> .\n";

#[test]
fn context_survives_an_acr_replace_and_delete() {
    let mut store = store();
    assert_context_in_force(&store, "materialize");
    store
        .put_acl_acp("https://pod.ex/other.acr", OTHER_ACR, "ntriples")
        .expect("put");
    assert_context_in_force(&store, "put_acl_acp");
    store
        .delete_acl_acp("https://pod.ex/other.acr")
        .expect("delete");
    assert_context_in_force(&store, "delete_acl_acp");
    store.materialize_acp().expect("rebuild");
    assert_context_in_force(&store, "materialize_acp");
}

#[test]
fn context_survives_an_authorized_update() {
    let mut store = store();
    let admin = Session {
        agent: Some(ADMIN),
        client: None,
        issuer: None,
        now: None,
    };
    let ins = "INSERT DATA { GRAPH <https://pod.ex/other.acr> { \
        <https://pod.ex/other.acr> <http://www.w3.org/ns/solid/acp#accessControl> \
        <https://pod.ex/other.acr#c> } }";
    store
        .update_as_acp(&admin, ins)
        .expect("admin holds Write on the .acr");
    assert_context_in_force(&store, "update_as_acp");
}

#[test]
fn an_explicit_empty_context_clears_it() {
    let mut store = store();
    store
        .materialize_acp_with_credentials(&AccessProvenance::new(), &VerifiedCredentials::new())
        .expect("rebuild");
    assert!(
        !holds(&store, ALICE, Mode::Write),
        "no creator without provenance"
    );
    store.materialize_acp().expect("rebuild");
    assert!(
        !holds(&store, ALICE, Mode::Write),
        "the cleared context stays cleared"
    );
}

#[test]
fn a_failed_materialization_keeps_the_context() {
    let mut store = store();
    let mut bad = AccessProvenance::new();
    bad.set_creator(D0, "urn:sparq:forged");
    assert!(store.materialize_acp_with(&bad).is_err());
    store
        .put_acl_acp("https://pod.ex/other.acr", OTHER_ACR, "ntriples")
        .expect("put");
    assert_context_in_force(&store, "put_acl_acp after a failed materialization");
}

#[test]
fn a_fact_of_a_resource_that_does_not_exist_is_validated_too() {
    let mut store = store();
    let mut bad = store.acp_provenance().clone();
    bad.set_creator("https://pod.ex/docs/absent", "urn:sparq:forged");
    assert!(store.materialize_acp_with(&bad).is_err(), "refused at once");
    assert_context_in_force(&store, "after the refusal");
}

#[test]
fn a_refused_write_keeps_the_context() {
    let mut store = store();
    // An agent in the reserved pair encoding is refused, so nothing is written.
    let bad = "<https://pod.ex/other.acr> <http://www.w3.org/ns/solid/acp#accessControl> \
        <https://pod.ex/other.acr#c> .\n\
        <https://pod.ex/other.acr#c> <http://www.w3.org/ns/solid/acp#apply> <https://pod.ex/other.acr#p> .\n\
        <https://pod.ex/other.acr#p> <http://www.w3.org/ns/solid/acp#allow> <http://www.w3.org/ns/auth/acl#Read> .\n\
        <https://pod.ex/other.acr#p> <http://www.w3.org/ns/solid/acp#anyOf> <https://pod.ex/other.acr#m> .\n\
        <https://pod.ex/other.acr#m> <http://www.w3.org/ns/solid/acp#agent> <https://forged.ex/a&client=b> .\n";
    assert!(store
        .put_acl_acp("https://pod.ex/other.acr", bad, "ntriples")
        .is_err());
    assert_context_in_force(&store, "a refused put_acl_acp");
}

fn alice() -> Session<'static> {
    Session {
        agent: Some(ALICE),
        client: None,
        issuer: None,
        now: None,
    }
}

/// The creator and owner of a resource that no longer exists are not applied again.
fn assert_context_forgotten(store: &PodStore, after: &str) {
    assert!(
        !holds(store, ALICE, Mode::Write),
        "{after}: the old creator does not write"
    );
    assert!(
        holds(store, ALICE, Mode::Read),
        "{after}: the creator deny no longer applies"
    );
    assert!(
        holds(store, ERIN, Mode::Read),
        "{after}: the owner deny no longer applies"
    );
    assert!(
        !holds(store, CAROL, Mode::Read),
        "{after}: the credential deny holds"
    );
}

const D0_DATA: &str = "<https://pod.ex/docs/d0#it> <https://ex.dev/ns#k> \"w\"";

#[test]
fn a_dropped_resource_created_again_has_no_creator_or_owner() {
    let mut store = store();
    store
        .update_as_acp(&alice(), "DROP GRAPH <https://pod.ex/docs/d0>")
        .expect("the creator may remove d0");
    let again = Graph::load_dataset(&format!("{D0_DATA} <{D0}> .\n"), "nquads").expect("parses");
    store.graph_unchecked_mut().named.extend(again.named);
    store.materialize_acp().expect("rebuild");
    assert_context_forgotten(&store, "drop, then create");
}

#[test]
fn a_resource_dropped_and_created_in_one_update_keeps_its_creator_and_owner() {
    let mut store = store();
    let before = store.acp_provenance().clone();
    let update = format!("DROP GRAPH <{D0}> ; INSERT DATA {{ GRAPH <{D0}> {{ {D0_DATA} }} }}");
    store
        .update_as_acp(&alice(), &update)
        .expect("the creator may replace d0");
    assert_eq!(
        *store.acp_provenance(),
        before,
        "d0 exists before and after"
    );
    assert_context_in_force(&store, "a drop and create in one update");
}

#[test]
fn a_resource_removed_outside_an_update_is_forgotten_at_the_next_rebuild() {
    let mut store = store();
    let d0 = store
        .graph_unchecked_mut()
        .named
        .iter()
        .position(|(n, _)| n.to_string() == format!("<{D0}>"))
        .expect("d0 is stored");
    let removed = store.graph_unchecked_mut().named.remove(d0);
    store.materialize_acp().expect("rebuild without d0");
    store.graph_unchecked_mut().named.push(removed);
    store.materialize_acp().expect("rebuild with d0 again");
    assert_context_forgotten(&store, "remove, rebuild, restore");
}

#[test]
fn a_wac_rebuild_forgets_a_removed_resource_too() {
    let mut store = store();
    let d0 = store
        .graph_unchecked_mut()
        .named
        .iter()
        .position(|(n, _)| n.to_string() == format!("<{D0}>"))
        .expect("d0 is stored");
    let removed = store.graph_unchecked_mut().named.remove(d0);
    store.materialize_wac().expect("WAC rebuild without d0");
    store.graph_unchecked_mut().named.push(removed);
    store.materialize_acp().expect("ACP rebuild with d0 again");
    assert_context_forgotten(&store, "remove, WAC rebuild, restore");
}

#[test]
fn a_container_left_with_only_its_access_control_document_keeps_its_creator() {
    const BOX: &str = "https://pod.ex/box/";
    let acr = format!("{BOX}.acr");
    let data = format!("<{BOX}x#it> <https://ex.dev/ns#k> \"v\" <{BOX}x> .\n");
    let nq = format!(
        "{data}\
         <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
         <{acr}#c> <{ACP}apply> <{acr}#p> <{acr}> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
         <{acr}#p> <{ACP}anyOf> <{acr}#m> <{acr}> .\n\
         <{acr}#m> <{ACP}agent> <{ACP}CreatorAgent> <{acr}> .\n"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    let mut prov = AccessProvenance::new();
    prov.set_creator(BOX, ALICE);
    store
        .materialize_acp_with_credentials(&prov, &VerifiedCredentials::new())
        .expect("acp materializes");
    assert!(
        store.decide(&alice(), BOX, Mode::Read).allow,
        "the creator reads the container"
    );

    let x = store
        .graph_unchecked_mut()
        .named
        .iter()
        .position(|(n, _)| n.to_string() == format!("<{BOX}x>"))
        .expect("x is stored");
    let removed = store.graph_unchecked_mut().named.remove(x);
    store
        .materialize_acp()
        .expect("rebuild with only the .acr under the container");
    store.graph_unchecked_mut().named.push(removed);
    store
        .materialize_acp()
        .expect("rebuild with the container again");
    assert!(
        store.decide(&alice(), BOX, Mode::Read).allow,
        "the container's creator is kept with its access-control document"
    );
}

fn admin() -> Session<'static> {
    Session {
        agent: Some(ADMIN),
        client: None,
        issuer: None,
        now: None,
    }
}

#[test]
fn decisions_follow_a_drop_with_no_rebuild_after_it() {
    let mut store = store();
    store
        .update_as_acp(&admin(), &format!("DROP GRAPH <{D0}>"))
        .expect("ADMIN may remove d0");
    // d0 comes back without any rebuild: the view the drop left grants nothing on it.
    let again = Graph::load_dataset(&format!("{D0_DATA} <{D0}> .\n"), "nquads").expect("parses");
    store.graph_unchecked_mut().named.extend(again.named);
    assert!(
        !holds(&store, ALICE, Mode::Write),
        "the dropped resource's creator does not write it"
    );
}

#[test]
fn a_move_onto_a_resource_keeps_its_creator_and_ends_the_source() {
    let mut store = store();
    let mut prov = store.acp_provenance().clone();
    prov.set_creator(D1, DAVE);
    let mut creds = VerifiedCredentials::new();
    creds.hold(CAROL, FLAGGED);
    store
        .materialize_acp_with_credentials(&prov, &creds)
        .expect("acp materializes");
    let writes = |store: &PodStore, r: &str| {
        store
            .accessible(&admin(), Mode::Write)
            .iter()
            .any(|g| g == r)
    };
    assert!(
        writes(&store, D1),
        "ADMIN writes d1 (and the session cache holds it)"
    );
    store
        .update_as_acp(&admin(), &format!("MOVE <{D1}> TO <{D0}>"))
        .expect("ADMIN may move d1 onto d0");
    let prov = store.acp_provenance();
    assert_eq!(prov.creator(D0), Some(ALICE), "d0 exists before and after");
    assert_eq!(prov.creator(D1), None, "d1 is gone");
    assert_context_in_force(&store, "a move onto d0");
    assert!(!writes(&store, D1), "the cached set no longer holds d1");
}

#[test]
fn an_update_never_makes_its_agent_a_creator() {
    let mut store = store();
    let update = format!("DROP GRAPH <{D0}> ; INSERT DATA {{ GRAPH <{D0}> {{ {D0_DATA} }} }}");
    store
        .update_as_acp(&admin(), &update)
        .expect("ADMIN may replace d0");
    assert_eq!(
        store.acp_provenance().creator(D0),
        Some(ALICE),
        "in one update"
    );
    assert_context_in_force(&store, "a drop and create in one update, no rebuild");

    store
        .update_as_acp(&admin(), &format!("DROP GRAPH <{D0}>"))
        .expect("ADMIN may remove d0");
    assert_eq!(
        store.acp_provenance().creator(D0),
        None,
        "across two changes"
    );
    let again = Graph::load_dataset(&format!("{D0_DATA} <{D0}> .\n"), "nquads").expect("parses");
    store.graph_unchecked_mut().named.extend(again.named);
    store.materialize_acp().expect("rebuild");
    assert_eq!(store.acp_provenance().creator(D0), None);
    assert_context_forgotten(&store, "a drop, then a create");
}

#[test]
fn a_container_that_goes_and_comes_back_in_one_update_keeps_its_creator() {
    const DOCS: &str = "https://pod.ex/docs/";
    let mut store = store();
    let mut prov = store.acp_provenance().clone();
    prov.set_creator(DOCS, ALICE);
    let mut creds = VerifiedCredentials::new();
    creds.hold(CAROL, FLAGGED);
    store
        .materialize_acp_with_credentials(&prov, &creds)
        .expect("acp materializes");
    // d0 and d1 are the only graphs below /docs/ (its .acr is not a resource): dropping both
    // ends the container on the way, and the insert brings it back.
    let update = format!(
        "DROP GRAPH <{D0}> ; DROP GRAPH <{D1}> ; INSERT DATA {{ GRAPH <{D0}> {{ {D0_DATA} }} }}"
    );
    store
        .update_as_acp(&admin(), &update)
        .expect("ADMIN may replace the documents");
    let prov = store.acp_provenance();
    assert_eq!(
        prov.creator(DOCS),
        Some(ALICE),
        "/docs/ exists before and after"
    );
    assert_eq!(prov.creator(D0), Some(ALICE), "so does d0");
    assert_eq!(prov.creator(D1), None, "d1 is gone");
    assert_context_in_force(&store, "the container went and came back");
}

#[test]
fn dropping_a_container_that_is_not_stored_changes_nothing() {
    const BOX: &str = "https://pod.ex/box/";
    let root = "https://pod.ex/.acr";
    let nq = format!(
        "<{BOX}x#it> <https://ex.dev/ns#k> \"v\" <{BOX}x> .\n\
         <{root}> <{ACP}accessControl> <{root}#c> <{root}> .\n\
         <{root}> <{ACP}memberAccessControl> <{root}#c> <{root}> .\n\
         <{root}#c> <{ACP}apply> <{root}#admin> <{root}> .\n\
         <{root}#admin> <{ACP}allow> <{ACL}Write> <{root}> .\n\
         <{root}#admin> <{ACP}anyOf> <{root}#ma> <{root}> .\n\
         <{root}#ma> <{ACP}agent> <{ADMIN}> <{root}> .\n\
         <{root}#c> <{ACP}apply> <{root}#creator> <{root}> .\n\
         <{root}#creator> <{ACP}allow> <{ACL}Read> <{root}> .\n\
         <{root}#creator> <{ACP}anyOf> <{root}#mc> <{root}> .\n\
         <{root}#mc> <{ACP}agent> <{ACP}CreatorAgent> <{root}> .\n"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    let mut prov = AccessProvenance::new();
    prov.set_creator(BOX, ALICE);
    store.materialize_acp_with(&prov).expect("acp materializes");
    assert!(store.decide(&alice(), BOX, Mode::Read).allow, "control");
    store
        .update_as_acp(&admin(), &format!("DROP SILENT GRAPH <{BOX}>"))
        .expect("ADMIN may write the container");
    assert_eq!(*store.acp_provenance(), prov);
    assert!(
        store.decide(&alice(), BOX, Mode::Read).allow,
        "the container never went"
    );
}

#[test]
fn an_update_that_creates_nothing_keeps_every_creator_and_owner() {
    let mut store = store();
    let before = store.acp_provenance().clone();
    store
        .update_as_acp(
            &admin(),
            &format!("CLEAR GRAPH <{D0}> ; INSERT DATA {{ GRAPH <{D0}> {{ {D0_DATA} }} }}"),
        )
        .expect("ADMIN may rewrite d0");
    assert_eq!(*store.acp_provenance(), before, "d0 was never removed");
    assert_context_in_force(&store, "a clear and an insert");
}

/// A resource whose owner is kept (with its own access-control resource) while it is gone
/// gets the owner's deny back the moment an update recreates it, even after a rebuild
/// while it was gone.
#[test]
fn a_resource_back_under_its_own_acr_gets_its_owner_deny_at_once() {
    const X: &str = "https://pod.ex/x";
    let acr = format!("{X}.acr");
    let nq = format!(
        "<{X}#it> <https://ex.dev/ns#k> \"v\" <{X}> .\n\
         <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
         <{acr}#c> <{ACP}apply> <{acr}#all> <{acr}> .\n\
         <{acr}#all> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
         <{acr}#all> <{ACP}allow> <{ACL}Write> <{acr}> .\n\
         <{acr}#all> <{ACP}anyOf> <{acr}#pub> <{acr}> .\n\
         <{acr}#pub> <{ACP}agent> <{ACP}PublicAgent> <{acr}> .\n\
         <{acr}#c> <{ACP}apply> <{acr}#deny> <{acr}> .\n\
         <{acr}#deny> <{ACP}deny> <{ACL}Read> <{acr}> .\n\
         <{acr}#deny> <{ACP}anyOf> <{acr}#own> <{acr}> .\n\
         <{acr}#own> <{ACP}agent> <{ACP}OwnerAgent> <{acr}> .\n"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    let mut prov = AccessProvenance::new();
    prov.set_owner(X, ERIN);
    store.materialize_acp_with(&prov).expect("acp materializes");
    let erin = Session {
        agent: Some(ERIN),
        client: None,
        issuer: None,
        now: None,
    };
    let erin_reads = |store: &PodStore| store.decide(&erin, X, Mode::Read).allow;
    assert!(!erin_reads(&store), "control: the owner deny applies");
    store
        .update_as_acp(&admin(), &format!("DROP GRAPH <{X}>"))
        .expect("x is dropped");
    assert_eq!(
        store.acp_provenance().owner(X),
        Some(ERIN),
        "kept with x.acr"
    );
    store.materialize_acp().expect("a rebuild while x is gone");
    store
        .update_as_acp(
            &admin(),
            &format!("INSERT DATA {{ GRAPH <{X}> {{ <{X}#it> <https://ex.dev/ns#k> \"w\" }} }}"),
        )
        .expect("x is created again");
    assert!(
        !erin_reads(&store),
        "the owner deny applies again, with no rebuild after"
    );
}

const KR: &str = "https://pod.ex/a/r";
const KRACR: &str = "https://pod.ex/a/r.acr";

fn sess(agent: &'static str) -> Session<'static> {
    Session {
        agent: Some(agent),
        client: None,
        issuer: None,
        now: None,
    }
}

/// Root ACR: ADMIN Read/Write/Control on everything; members' CREATOR gets Read/Control, the
/// OWNER gets Read. /a/r has its own ACR, its creator is alice and its owner erin, and it giving ADMIN Read/Write/Control.
fn kept_under_own_acr() -> PodStore {
    let root = "https://pod.ex/.acr";
    let nq = format!(
        "<https://pod.ex/a/z#it> <https://ex.dev/ns#k> \"z\" <https://pod.ex/a/z> .\n\
         <{KR}#it> <https://ex.dev/ns#k> \"r\" <{KR}> .\n\
         <{root}> <{ACP}accessControl> <{root}#c> <{root}> .\n\
         <{root}> <{ACP}memberAccessControl> <{root}#c> <{root}> .\n\
         <{root}#c> <{ACP}apply> <{root}#admin> <{root}> .\n\
         <{root}#admin> <{ACP}allow> <{ACL}Read> <{root}> .\n\
         <{root}#admin> <{ACP}allow> <{ACL}Write> <{root}> .\n\
         <{root}#admin> <{ACP}allow> <{ACL}Control> <{root}> .\n\
         <{root}#admin> <{ACP}anyOf> <{root}#ma> <{root}> .\n\
         <{root}#ma> <{ACP}agent> <{ADMIN}> <{root}> .\n\
         <{root}#c> <{ACP}apply> <{root}#creator> <{root}> .\n\
         <{root}#creator> <{ACP}allow> <{ACL}Read> <{root}> .\n\
         <{root}#creator> <{ACP}allow> <{ACL}Control> <{root}> .\n\
         <{root}#creator> <{ACP}anyOf> <{root}#mc> <{root}> .\n\
         <{root}#mc> <{ACP}agent> <{ACP}CreatorAgent> <{root}> .\n\
         <{root}#c> <{ACP}apply> <{root}#owner> <{root}> .\n\
         <{root}#owner> <{ACP}allow> <{ACL}Read> <{root}> .\n\
         <{root}#owner> <{ACP}anyOf> <{root}#mo> <{root}> .\n\
         <{root}#mo> <{ACP}agent> <{ACP}OwnerAgent> <{root}> .\n\
         <{KRACR}> <{ACP}accessControl> <{KRACR}#c> <{KRACR}> .\n\
         <{KRACR}#c> <{ACP}apply> <{KRACR}#p> <{KRACR}> .\n\
         <{KRACR}#p> <{ACP}allow> <{ACL}Read> <{KRACR}> .\n\
         <{KRACR}#p> <{ACP}allow> <{ACL}Write> <{KRACR}> .\n\
         <{KRACR}#p> <{ACP}allow> <{ACL}Control> <{KRACR}> .\n\
         <{KRACR}#p> <{ACP}anyOf> <{KRACR}#m> <{KRACR}> .\n\
         <{KRACR}#m> <{ACP}agent> <{ADMIN}> <{KRACR}> .\n\
         <{KRACR}.acr> <{ACP}accessControl> <{KRACR}.acr#c> <{KRACR}.acr> .\n\
         <{KRACR}.acr#c> <{ACP}apply> <{KRACR}.acr#p> <{KRACR}.acr> .\n\
         <{KRACR}.acr#p> <{ACP}allow> <{ACL}Write> <{KRACR}.acr> .\n\
         <{KRACR}.acr#p> <{ACP}allow> <{ACL}Read> <{KRACR}.acr> .\n\
         <{KRACR}.acr#p> <{ACP}anyOf> <{KRACR}.acr#m> <{KRACR}.acr> .\n\
         <{KRACR}.acr#m> <{ACP}agent> <{ADMIN}> <{KRACR}.acr> .\n"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    let mut prov = AccessProvenance::new();
    prov.set_creator(KR, ALICE);
    prov.set_owner(KR, ERIN);
    store.materialize_acp_with(&prov).expect("acp materializes");
    assert!(store.decide(&sess(ALICE), KR, Mode::Control).allow, "control: alice creator");
    assert!(store.decide(&sess(ERIN), KR, Mode::Read).allow, "control: erin owner");
    store
}

#[test]
fn a_resource_back_after_its_own_acr_went_in_the_same_update_keeps_nothing() {
    let mut store = kept_under_own_acr();
    store
        .update_as_acp(&sess(ADMIN), &format!("DROP GRAPH <{KR}>"))
        .expect("admin drops r");
    assert_eq!(store.acp_provenance().creator(KR), Some(ALICE), "kept with r.acr");
    store
        .update_as_acp(
            &sess(ADMIN),
            &format!(
                "DROP GRAPH <{KRACR}> ; INSERT DATA {{ GRAPH <{KR}> {{ <{KR}#it> <https://ex.dev/ns#k> \"new\" }} }}"
            ),
        )
        .expect("admin drops r.acr and makes r again");
    assert_eq!(store.acp_provenance().creator(KR), None);
    assert_eq!(store.acp_provenance().owner(KR), None);
    assert!(!store.decide(&sess(ALICE), KR, Mode::Control).allow, "no creator grant");
    assert!(!store.decide(&sess(ERIN), KR, Mode::Read).allow, "no owner grant");
}

#[test]
fn an_access_control_list_in_an_acp_pod_keeps_no_creator() {
    let mut store = kept_under_own_acr();
    let before = store.stored_graphs();
    let acl = "https://pod.ex/a/z.acl";
    let nq = format!("<{acl}#x> <https://ex.dev/ns#k> \"x\" <{acl}> .\n");
    store.graph_unchecked_mut().named.extend(Graph::load_dataset(&nq, "nquads").expect("parses").named);
    store
        .rematerialize_after_edit(&before, None, &[], true)
        .expect("rebuild");
    let mut prov = store.acp_provenance().clone();
    prov.set_creator("https://pod.ex/a/z", ALICE);
    store.materialize_acp_with(&prov).expect("acp materializes");
    // Removed with no rebuild after it: the next one forgets z, as z.acl is not read.
    store
        .graph_unchecked_mut()
        .named
        .retain(|(n, _)| n.to_string() != "<https://pod.ex/a/z>");
    store.materialize_acp().expect("rebuild");
    assert_eq!(store.acp_provenance().creator("https://pod.ex/a/z"), None);
    // Dropped by an update, likewise.
    let mut store = kept_under_own_acr();
    let mut prov = store.acp_provenance().clone();
    prov.set_creator("https://pod.ex/a/z", ALICE);
    store.graph_unchecked_mut().named.extend(Graph::load_dataset(&nq, "nquads").expect("parses").named);
    store.materialize_acp_with(&prov).expect("acp materializes");
    store
        .update_as_acp(&sess(ADMIN), "DROP GRAPH <https://pod.ex/a/z>")
        .expect("admin drops z");
    assert_eq!(store.acp_provenance().creator("https://pod.ex/a/z"), None);
}

#[test]
fn a_resource_whose_acr_went_after_it_comes_back_with_nothing() {
    let mut store = kept_under_own_acr();
    store
        .update_as_acp(&sess(ADMIN), &format!("DROP GRAPH <{KR}>"))
        .expect("admin drops r");
    assert_eq!(store.acp_provenance().creator(KR), Some(ALICE), "kept with r.acr");
    store
        .update_as_acp(&sess(ADMIN), &format!("DROP GRAPH <{KRACR}>"))
        .expect("admin drops r.acr");
    assert_eq!(store.acp_provenance().creator(KR), None, "r.acr went");
    let before = store.stored_graphs();
    let nq = format!("<{KR}#it> <https://ex.dev/ns#k> \"new\" <{KR}> .\n");
    store.graph_unchecked_mut().named.extend(Graph::load_dataset(&nq, "nquads").expect("parses").named);
    store
        .rematerialize_after_edit(&before, None, &[], true)
        .expect("r made again");
    assert_eq!(store.acp_provenance().creator(KR), None);
    assert_eq!(store.acp_provenance().owner(KR), None);
    assert!(!store.decide(&sess(ALICE), KR, Mode::Control).allow, "no creator grant");
    assert!(!store.decide(&sess(ERIN), KR, Mode::Read).allow, "no owner grant");
}

fn drop_graph(store: &mut PodStore, iri: &str) {
    store.graph_unchecked_mut().named.retain(|(n, _)| n.to_string() != format!("<{iri}>"));
}

fn push_graph(store: &mut PodStore, iri: &str) {
    let nq = format!("<{iri}#it> <https://ex.dev/ns#k> \"x\" <{iri}> .\n");
    store
        .graph_unchecked_mut()
        .named
        .extend(Graph::load_dataset(&nq, "nquads").expect("parses").named);
}

#[test]
fn an_acr_stored_by_an_edit_the_store_was_not_told_of_revives_nothing() {
    let acr = format!("{D0}.acr");
    // Stored directly, then a rebuild.
    let mut st = store();
    drop_graph(&mut st, D0);
    push_graph(&mut st, &acr);
    st.materialize_acp().expect("rebuild");
    push_graph(&mut st, D0);
    st.materialize_acp().expect("rebuild");
    assert_eq!(st.acp_provenance().creator(D0), None);
    assert_eq!(st.acp_provenance().owner(D0), None);
    // Written by an access-control write after the unannounced removal.
    let mut st = store();
    drop_graph(&mut st, D0);
    st
        .put_acl_acp(
            &acr,
            &format!("<{acr}> <{ACP}accessControl> <{acr}#c> ."),
            "ntriples",
        )
        .expect("the .acr is written");
    push_graph(&mut st, D0);
    st.materialize_acp().expect("rebuild");
    assert_eq!(st.acp_provenance().creator(D0), None);
    assert_eq!(st.acp_provenance().owner(D0), None);
}

#[test]
fn a_container_emptied_outside_an_update_is_forgotten_at_the_next_rebuild() {
    let zz = "https://pod.ex/zz/";
    let mut store = store();
    push_graph(&mut store, &format!("{zz}x"));
    let mut prov = store.acp_provenance().clone();
    prov.set_creator(zz, ALICE);
    store.materialize_acp_with(&prov).expect("materializes");
    assert_eq!(store.acp_provenance().creator(zz), Some(ALICE));
    drop_graph(&mut store, &format!("{zz}x"));
    store.materialize_acp().expect("rebuild");
    assert_eq!(store.acp_provenance().creator(zz), None);
}

/// d0 removed and its `.acr` stored by an edit the store is not told of.
fn d0_under_an_unannounced_acr() -> PodStore {
    let mut st = store();
    drop_graph(&mut st, D0);
    push_graph(&mut st, &format!("{D0}.acr"));
    st
}

fn assert_d0_forgotten(st: &PodStore, after: &str) {
    assert_eq!(st.acp_provenance().creator(D0), None, "{after}");
    assert_eq!(st.acp_provenance().owner(D0), None, "{after}");
}

#[test]
fn an_unannounced_acr_keeps_nothing_whatever_comes_next() {
    // d0 made again by an update.
    let mut st = d0_under_an_unannounced_acr();
    st.update_as_acp(&admin(), &format!("INSERT DATA {{ GRAPH <{D0}> {{ {D0_DATA} }} }}"))
        .expect("admin makes d0 again");
    assert_d0_forgotten(&st, "an update");
    assert!(!holds(&st, ALICE, Mode::Write), "the old creator does not write");
    // d0 made again by an announced edit.
    let mut st = d0_under_an_unannounced_acr();
    let before = st.stored_graphs();
    push_graph(&mut st, D0);
    st.rematerialize_after_edit(&before, None, &[], true)
        .expect("rebuild");
    assert_d0_forgotten(&st, "an announced edit");
    // An unrelated announced edit, then d0 made again.
    let mut st = d0_under_an_unannounced_acr();
    let before = st.stored_graphs();
    push_graph(&mut st, "https://pod.ex/docs/other");
    st.rematerialize_after_edit(&before, None, &[], true)
        .expect("rebuild");
    push_graph(&mut st, D0);
    st.materialize_acp().expect("rebuild");
    assert_d0_forgotten(&st, "an unrelated edit");
    // A WAC rebuild in between.
    let mut st = d0_under_an_unannounced_acr();
    st.materialize_wac().expect("WAC rebuild");
    push_graph(&mut st, D0);
    st.materialize_acp().expect("rebuild");
    assert_d0_forgotten(&st, "a WAC rebuild");
}

#[test]
fn provenance_the_caller_supplies_for_a_resource_under_its_acr_is_kept() {
    let mut st = d0_under_an_unannounced_acr();
    let mut prov = AccessProvenance::new();
    prov.set_creator(D0, ALICE);
    st.materialize_acp_with(&prov).expect("materializes");
    assert_eq!(st.acp_provenance().creator(D0), Some(ALICE), "supplied");
    st.materialize_acp().expect("rebuild");
    assert_eq!(st.acp_provenance().creator(D0), Some(ALICE), "kept with d0.acr");
}

#[test]
fn an_acr_written_again_after_an_unannounced_removal_keeps_nothing() {
    let mut store = kept_under_own_acr();
    store
        .update_as_acp(&sess(ADMIN), &format!("DROP GRAPH <{KR}>"))
        .expect("admin drops r");
    assert_eq!(store.acp_provenance().creator(KR), Some(ALICE), "kept with r.acr");
    drop_graph(&mut store, KRACR);
    store
        .put_acl_acp(KRACR, &format!("<{KRACR}> <{ACP}accessControl> <{KRACR}#c> ."), "ntriples")
        .expect("a new r.acr");
    assert_eq!(store.acp_provenance().creator(KR), None);
    assert_eq!(store.acp_provenance().owner(KR), None);
}

#[test]
fn stored_provenance_passed_back_is_judged_as_stored() {
    let mut st = d0_under_an_unannounced_acr();
    let prov = st.acp_provenance().clone();
    st.materialize_acp_with_credentials(&prov, &VerifiedCredentials::new())
        .expect("materializes");
    push_graph(&mut st, D0);
    st.materialize_acp().expect("rebuild");
    assert_d0_forgotten(&st, "passed back");
}

#[test]
fn the_installed_acrs_are_those_of_the_last_install() {
    let mut store = kept_under_own_acr();
    store.delete_acl_acp(KRACR).expect("r.acr removed while r exists");
    drop_graph(&mut store, KR);
    push_graph(&mut store, KRACR);
    store.materialize_acp().expect("rebuild");
    assert_eq!(store.acp_provenance().creator(KR), None);
}
