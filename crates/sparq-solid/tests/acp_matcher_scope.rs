//! A provenance matcher (`acp:CreatorAgent`, `acp:OwnerAgent`) is decided per resource in
//! every role: allOf, anyOf and noneOf. Being the creator or owner of one resource never
//! satisfies, or excepts from, a policy on another.

use sparq_core::Graph;
use sparq_solid::{AccessProvenance, Mode, PodStore, Session};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const DOCS: &str = "https://pod.ex/docs/.acr";
const R1: &str = "https://pod.ex/docs/r1";
const R2: &str = "https://pod.ex/docs/r2";
const R3: &str = "https://pod.ex/docs/r3";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const DAVE: &str = "https://dave.ex/card#me";

/// `/docs/` members get the policies `pols` (each `(name, triples about <DOCS#name>)`),
/// under `prov`.
fn store(policies: &[(&str, &str)], prov: &AccessProvenance) -> PodStore {
    let mut nq = String::new();
    for r in [R1, R2, R3] {
        nq += &format!("<{r}#it> <https://ex.dev/ns#k> \"v\" <{r}> .\n");
    }
    nq += &format!("<{DOCS}> <{ACP}memberAccessControl> <{DOCS}#c> <{DOCS}> .\n");
    for (name, body) in policies {
        nq += &format!("<{DOCS}#c> <{ACP}apply> <{DOCS}#{name}> <{DOCS}> .\n");
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            nq += &format!("{} <{DOCS}> .\n", line.trim().trim_end_matches('.').trim());
        }
    }
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("fixture loads"));
    store.materialize_acp_with(prov).expect("acp materializes");
    store
}

fn reads(store: &PodStore, agent: &str, r: &str) -> bool {
    let s = Session {
        agent: Some(agent),
        client: None,
        issuer: None,
        now: None,
    };
    store.decide(&s, r, Mode::Read).allow
}

fn matcher(name: &str, agent: &str) -> String {
    format!("<{DOCS}#{name}> <{ACP}agent> <{agent}>\n")
}

#[test]
fn all_of_creator_and_owner_needs_both_on_the_same_resource() {
    let mut prov = AccessProvenance::new();
    prov.set_creator(R1, ALICE);
    prov.set_owner(R2, ALICE);
    prov.set_creator(R3, BOB);
    prov.set_owner(R3, BOB);
    let pol = format!(
        "<{DOCS}#p> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#p> <{ACP}allOf> <{DOCS}#mc>\n\
         <{DOCS}#p> <{ACP}allOf> <{DOCS}#mo>\n{}{}",
        matcher("mc", &format!("{ACP}CreatorAgent")),
        matcher("mo", &format!("{ACP}OwnerAgent")),
    );
    let s = store(&[("p", &pol)], &prov);
    assert!(reads(&s, BOB, R3), "creator and owner of r3");
    assert!(!reads(&s, ALICE, R1), "creator of r1, owner of r2 only");
    assert!(!reads(&s, ALICE, R2), "owner of r2, creator of r1 only");
}

#[test]
fn any_of_creator_counts_only_on_the_created_resource() {
    let mut prov = AccessProvenance::new();
    prov.set_creator(R2, ALICE);
    let pol = format!(
        "<{DOCS}#p> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#p> <{ACP}allOf> <{DOCS}#ma>\n\
         <{DOCS}#p> <{ACP}anyOf> <{DOCS}#mc>\n{}{}",
        matcher("ma", ALICE),
        matcher("mc", &format!("{ACP}CreatorAgent")),
    );
    let s = store(&[("p", &pol)], &prov);
    assert!(reads(&s, ALICE, R2), "alice created r2");
    assert!(!reads(&s, ALICE, R1), "alice did not create r1");
}

#[test]
fn none_of_creator_excepts_the_creator_of_that_resource() {
    let mut prov = AccessProvenance::new();
    prov.set_creator(R1, ALICE);
    let public = format!("{ACP}PublicAgent");
    // Everyone reads, except each resource's creator.
    let allow = format!(
        "<{DOCS}#p> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#p> <{ACP}anyOf> <{DOCS}#pub>\n\
         <{DOCS}#p> <{ACP}noneOf> <{DOCS}#mc>\n{}{}",
        matcher("pub", &public),
        matcher("mc", &format!("{ACP}CreatorAgent")),
    );
    let s = store(&[("p", &allow)], &prov);
    assert!(!reads(&s, ALICE, R1), "the creator of r1 is excepted on r1");
    assert!(reads(&s, ALICE, R2), "but not on r2");
    assert!(reads(&s, DAVE, R1), "everyone else reads r1");

    // Everyone reads; everyone but each resource's owner is denied.
    let mut prov = AccessProvenance::new();
    prov.set_owner(R1, ALICE);
    let read = format!(
        "<{DOCS}#all> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#all> <{ACP}anyOf> <{DOCS}#pub>\n{}",
        matcher("pub", &public),
    );
    let deny = format!(
        "<{DOCS}#d> <{ACP}deny> <{ACL}Read>\n\
         <{DOCS}#d> <{ACP}anyOf> <{DOCS}#pub>\n\
         <{DOCS}#d> <{ACP}noneOf> <{DOCS}#mo>\n{}",
        matcher("mo", &format!("{ACP}OwnerAgent")),
    );
    let s = store(&[("all", &read), ("d", &deny)], &prov);
    assert!(reads(&s, ALICE, R1), "the owner of r1 is excepted from the deny on r1");
    assert!(!reads(&s, ALICE, R2), "but not on r2");
    assert!(!reads(&s, DAVE, R1), "everyone else is denied r1");
}
