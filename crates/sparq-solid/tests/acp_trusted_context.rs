//! The trusted ACP context (creator/owner provenance and verified credentials) a store was
//! materialized with stays in force across the rebuilds the store makes itself: after an
//! authoritative `.acr` replace or delete, and after an authorized update, the grants and
//! denies that context decides are unchanged. A resource that goes keeps none of it: one
//! created again under the same IRI has no creator or owner.

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
fn a_rolled_back_write_keeps_the_context() {
    let mut store = store();
    // An agent in the reserved pair encoding cannot be materialized, so the write rolls back.
    let bad = "<https://pod.ex/other.acr> <http://www.w3.org/ns/solid/acp#accessControl> \
        <https://pod.ex/other.acr#c> .\n\
        <https://pod.ex/other.acr#c> <http://www.w3.org/ns/solid/acp#apply> <https://pod.ex/other.acr#p> .\n\
        <https://pod.ex/other.acr#p> <http://www.w3.org/ns/solid/acp#allow> <http://www.w3.org/ns/auth/acl#Read> .\n\
        <https://pod.ex/other.acr#p> <http://www.w3.org/ns/solid/acp#anyOf> <https://pod.ex/other.acr#m> .\n\
        <https://pod.ex/other.acr#m> <http://www.w3.org/ns/solid/acp#agent> <https://forged.ex/a&client=b> .\n";
    assert!(store
        .put_acl_acp("https://pod.ex/other.acr", bad, "ntriples")
        .is_err());
    assert_context_in_force(&store, "a rolled-back put_acl_acp");
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
    store.graph.named.extend(again.named);
    store.materialize_acp().expect("rebuild");
    assert_context_forgotten(&store, "drop, then create");
}

#[test]
fn a_resource_dropped_and_created_in_one_update_has_no_creator_or_owner() {
    let mut store = store();
    let update = format!("DROP GRAPH <{D0}> ; INSERT DATA {{ GRAPH <{D0}> {{ {D0_DATA} }} }}");
    store
        .update_as_acp(&alice(), &update)
        .expect("the creator may replace d0");
    store.materialize_acp().expect("rebuild");
    assert_context_forgotten(&store, "drop and create in one update");
}

#[test]
fn a_resource_removed_outside_an_update_is_forgotten_at_the_next_rebuild() {
    let mut store = store();
    let d0 = store
        .graph
        .named
        .iter()
        .position(|(n, _)| n.to_string() == format!("<{D0}>"))
        .expect("d0 is stored");
    let removed = store.graph.named.remove(d0);
    store.materialize_acp().expect("rebuild without d0");
    store.graph.named.push(removed);
    store.materialize_acp().expect("rebuild with d0 again");
    assert_context_forgotten(&store, "remove, rebuild, restore");
}

#[test]
fn a_container_left_with_only_its_access_control_document_is_forgotten() {
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
        .graph
        .named
        .iter()
        .position(|(n, _)| n.to_string() == format!("<{BOX}x>"))
        .expect("x is stored");
    let removed = store.graph.named.remove(x);
    store
        .materialize_acp()
        .expect("rebuild with only the .acr under the container");
    store.graph.named.push(removed);
    store
        .materialize_acp()
        .expect("rebuild with the container again");
    assert!(
        !store.decide(&alice(), BOX, Mode::Read).allow,
        "a container that held only its access-control document was gone"
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
    store.graph.named.extend(again.named);
    assert!(
        !holds(&store, ALICE, Mode::Write),
        "the dropped resource's creator does not write it"
    );
}

#[test]
fn decisions_follow_a_move_onto_a_resource_with_no_rebuild_after_it() {
    let mut store = store();
    let writes_d0 = |store: &PodStore| {
        store
            .accessible(&alice(), Mode::Write)
            .iter()
            .any(|g| g == D0)
    };
    assert!(
        writes_d0(&store),
        "the creator writes d0 (and the session cache holds it)"
    );
    store
        .update_as_acp(&admin(), &format!("MOVE <{D1}> TO <{D0}>"))
        .expect("ADMIN may move d1 onto d0");
    assert_context_forgotten(&store, "move onto d0");
    assert!(!writes_d0(&store), "the cached set no longer holds d0");
}

#[test]
fn decisions_follow_a_drop_and_create_in_one_update() {
    let mut store = store();
    let update = format!("DROP GRAPH <{D0}> ; INSERT DATA {{ GRAPH <{D0}> {{ {D0_DATA} }} }}");
    store
        .update_as_acp(&admin(), &update)
        .expect("ADMIN may replace d0");
    assert_context_forgotten(&store, "drop and create in one update, no rebuild");
}

#[test]
fn a_provenance_change_through_the_store_applies_to_the_next_decision() {
    let mut store = store();
    let dave = Session {
        agent: Some(DAVE),
        client: None,
        issuer: None,
        now: None,
    };
    let dave_writes_d1 =
        |store: &PodStore| store.accessible(&dave, Mode::Write).iter().any(|g| g == D1);
    assert!(
        !dave_writes_d1(&store),
        "dave writes nothing yet (and the session cache holds that)"
    );
    assert!(
        store.record_creator(D1, "urn:sparq:forged").is_err(),
        "not a principal"
    );
    store.record_creator(D1, DAVE).expect("a WebID");
    assert!(
        store.decide(&dave, D1, Mode::Write).allow,
        "the recorded creator writes d1"
    );
    assert!(dave_writes_d1(&store), "and the cached set follows");
    assert_context_in_force(&store, "the earlier provenance is kept");

    let mut without_alice = store.acp_provenance().clone();
    without_alice.forget(D0);
    let previous = store
        .replace_acp_provenance(without_alice)
        .expect("rebuild");
    assert_context_forgotten(&store, "a replaced provenance, no explicit rebuild");
    store.replace_acp_provenance(previous).expect("rebuild");
    assert_context_in_force(&store, "the restored provenance");
}
