//! The trusted ACP context (creator/owner provenance and verified credentials) a store was
//! materialized with stays in force across the rebuilds the store makes itself: after an
//! authoritative `.acr` replace or delete, and after an authorized update, the grants and
//! denies that context decides are unchanged.

use sparq_core::Graph;
use sparq_solid::{AccessProvenance, Mode, PodStore, Session, VerifiedCredentials};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const D0: &str = "https://pod.ex/docs/d0";
const ALICE: &str = "https://alice.ex/card#me"; // created d0
const ERIN: &str = "https://erin.ex/card#me"; // owns d0
const CAROL: &str = "https://carol.ex/card#me"; // holds the flagged credential
const DAVE: &str = "https://dave.ex/card#me"; // no context at all
const ADMIN: &str = "https://admin.ex/card#me";
const FLAGGED: &str = "https://issuer.ex/req#Flagged";

/// `/docs/` policies: everyone reads, the creator also writes, and the owner and holders
/// of the flagged credential are denied Read. `/other.acr` is unrelated; ADMIN holds Write
/// on it through `/other.acr.acr`.
fn store() -> PodStore {
    let docs = "https://pod.ex/docs/.acr";
    let mut nq = format!(
        "<{D0}#it> <https://ex.dev/ns#k> \"v\" <{D0}> .\n\
         <https://pod.ex/other#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/other> .\n"
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
