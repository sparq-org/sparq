//! What the trusted creator/owner facts do when a resource's own graph comes and goes, and
//! what the ACP view makes of a policy or matcher reference it cannot resolve.
//!
//! Two behaviours are pinned here because an ACP view that reads `acp:CreatorAgent` turns
//! them into grants:
//!
//! - a container that still has members is a resource whether or not its own graph is
//!   stored, so removing and storing that graph again leaves its creator and owner as they
//!   were (nobody becomes its creator by writing it again);
//! - a resource brought into being brings its missing ancestor containers into being with it,
//!   and those containers keep no creator unless the caller names them, so a writer never
//!   becomes the creator of a container above what it wrote.

use sparq_core::Graph;
use sparq_solid::{AccessProvenance, Mode, PodStore, Session};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";

fn session(agent: &str) -> Session<'_> {
    Session {
        agent: Some(agent),
        ..Session::default()
    }
}

/// `/a/` holds one member `/a/x` and its own graph, under an `.acr` whose member policy
/// gives every creator Write and Control.
fn pod() -> String {
    let acr = "https://pod.ex/a/.acr";
    format!(
        "<https://pod.ex/a/x#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/a/x> .\n\
         <https://pod.ex/a/#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/a/> .\n\
         <{acr}> <{ACP}memberAccessControl> <{acr}#c> <{acr}> .\n\
         <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
         <{acr}#c> <{ACP}apply> <{acr}#creator> <{acr}> .\n\
         <{acr}#creator> <{ACP}allow> <{ACL}Write> <{acr}> .\n\
         <{acr}#creator> <{ACP}allow> <{ACL}Control> <{acr}> .\n\
         <{acr}#creator> <{ACP}anyOf> <{acr}#mc> <{acr}> .\n\
         <{acr}#mc> <{ACP}agent> <{ACP}CreatorAgent> <{acr}> .\n"
    )
}

fn store_with_creator(resource: &str, who: &str) -> PodStore {
    let mut store = PodStore::new(Graph::load_dataset(&pod(), "nquads").expect("parses"));
    let mut prov = AccessProvenance::new();
    prov.set_creator(resource, who);
    store.materialize_acp_with(&prov).expect("materializes");
    store
}

/// A container that still holds a member is a resource whether or not its own graph is
/// stored, so a writer who removes that graph and stores it again does not become the
/// container's creator — and gains neither the Write nor the Control its `.acr` gives
/// creators.
#[test]
fn a_container_with_members_keeps_its_creator_when_its_own_graph_is_stored_again() {
    let container = "https://pod.ex/a/";
    let mut store = store_with_creator(container, ALICE);
    assert_eq!(store.acp_provenance().creator(container), Some(ALICE));

    // Remove the container's own graph; `/a/x` keeps it a resource.
    let before = store.stored_graphs();
    let own = store
        .graph()
        .named()
        .iter()
        .position(|(n, _)| n.to_string() == format!("<{container}>"))
        .expect("the container's own graph is stored");
    let removed = {
        let g = store.graph_unchecked_mut();
        let name = g.named()[own].0.clone();
        let sub = g.remove_named(&name).unwrap().expect("present");
        (name, sub)
    };
    store
        .rematerialize_after_edit(&before, Some(BOB), &[], true)
        .expect("rebuilds");
    assert_eq!(
        store.acp_provenance().creator(container),
        Some(ALICE),
        "the container still has a member, so it never stopped being a resource"
    );

    // Store it again, as a writer holding Write on `/a/` would.
    let before = store.stored_graphs();
    store.graph_unchecked_mut().add_named(removed.0, removed.1).unwrap();
    store
        .rematerialize_after_edit(&before, Some(BOB), &[], true)
        .expect("rebuilds");
    assert_eq!(
        store.acp_provenance().creator(container),
        Some(ALICE),
        "writing the container's own graph again does not make its writer the creator"
    );
    assert!(
        !store.decide(&session(BOB), container, Mode::Control).allow,
        "the creator policy still answers for alice"
    );
    assert!(store.decide(&session(ALICE), container, Mode::Control).allow);
}

/// The creator is recorded only for the resources the caller NAMES. A write stored below
/// containers that do not exist yet brings each of those containers into being, and they keep
/// no creator, so a writer never becomes the creator of a container above what it wrote. A
/// caller that names such a container does record it — the facts are the caller's to assert.
#[test]
fn only_the_resources_a_caller_names_record_its_agent_as_creator() {
    let deep = "https://pod.ex/a/n/deep/doc";
    let nq = format!("<{deep}#it> <https://ex.dev/ns#k> \"v\" <{deep}> .\n");

    let mut store = PodStore::new(Graph::load_dataset(&pod(), "nquads").expect("parses"));
    store.materialize_acp().expect("materializes");
    let before = store.stored_graphs();
    let mut added = Graph::load_dataset(&nq, "nquads").expect("parses");
    store.graph_unchecked_mut().extend_named(added.take_named().unwrap()).unwrap();
    store
        .rematerialize_after_edit(&before, Some(BOB), &[deep], true)
        .expect("rebuilds");

    let prov = store.acp_provenance();
    assert_eq!(prov.creator(deep), Some(BOB), "the resource he named");
    for container in ["https://pod.ex/a/n/deep/", "https://pod.ex/a/n/"] {
        assert_eq!(
            prov.creator(container),
            None,
            "a container the write brought into being, not named: {}",
            container
        );
    }
    assert_eq!(
        prov.creator("https://pod.ex/a/"),
        None,
        "/a/ existed already, so the write changed nothing about it"
    );
    assert!(
        !store
            .decide(&session(BOB), "https://pod.ex/a/n/", Mode::Control)
            .allow,
        "the creator policy answers for nobody on a container the write brought along"
    );

    // Named, the same container does record him: what a caller asserts is its own.
    let mut store = PodStore::new(Graph::load_dataset(&pod(), "nquads").expect("parses"));
    store.materialize_acp().expect("materializes");
    let before = store.stored_graphs();
    let mut added = Graph::load_dataset(&nq, "nquads").expect("parses");
    store.graph_unchecked_mut().extend_named(added.take_named().unwrap()).unwrap();
    store
        .rematerialize_after_edit(&before, Some(BOB), &[deep, "https://pod.ex/a/n/"], true)
        .expect("rebuilds");
    assert_eq!(
        store.acp_provenance().creator("https://pod.ex/a/n/"),
        Some(BOB)
    );
}

/// A policy or matcher reference the view cannot resolve grants nothing: an `.acr` naming a
/// policy with no statements, a policy whose only matcher has no statements, a policy with no
/// condition at all, and a reference into a document that is not stored each leave the
/// resource unreadable — by its own agent and by anyone.
#[test]
fn an_unresolved_policy_or_matcher_reference_grants_nothing() {
    let doc = "https://pod.ex/u/r";
    let acr = "https://pod.ex/u/r.acr";
    let base = format!(
        "<{doc}#it> <https://ex.dev/ns#k> \"v\" <{doc}> .\n\
         <{acr}> <{ACP}resource> <{doc}> <{acr}> .\n\
         <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n"
    );
    let cases = [
        // The applied policy has no statements at all.
        format!("<{acr}#c> <{ACP}apply> <{acr}#missing> <{acr}> .\n"),
        // The applied policy allows Read, but its only matcher has no statements.
        format!(
            "<{acr}#c> <{ACP}apply> <{acr}#p> <{acr}> .\n\
             <{acr}#p> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
             <{acr}#p> <{ACP}anyOf> <{acr}#nomatcher> <{acr}> .\n"
        ),
        // The applied policy allows Read under no condition whatsoever.
        format!(
            "<{acr}#c> <{ACP}apply> <{acr}#p> <{acr}> .\n\
             <{acr}#p> <{ACP}allow> <{ACL}Read> <{acr}> .\n"
        ),
        // The applied policy lives in a document that is not stored.
        format!("<{acr}#c> <{ACP}apply> <https://pod.ex/u/absent.acr#p> <{acr}> .\n"),
    ];
    for (i, case) in cases.iter().enumerate() {
        let nq = format!("{base}{case}");
        let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
        store.materialize_acp().expect("materializes");
        for agent in [Some(ALICE), None] {
            let s = Session {
                agent,
                ..Session::default()
            };
            assert!(
                !store.decide(&s, doc, Mode::Read).allow,
                "case {}: an unresolved reference granted Read",
                i
            );
            assert!(
                !store.accessible(&s, Mode::Read).iter().any(|g| g.as_str() == doc),
                "case {}: an unresolved reference put the document in the readable set",
                i
            );
        }
    }
}
