//! The bridge's denies cover every request `decide` denies.
//!
//! Prohibitions are generated over every constraint left operand the evaluator knows
//! (and one it does not), each assignee shape, and compound constraints, then layered
//! over a public WAC read grant through the one-shot and the conditional deny entry
//! points, for each requesting party. After materialization and again after a ledger
//! refresh:
//!
//! - the requesting party is denied whenever a prohibition still applies to its request
//!   (`matched_prohibition`, which keeps a prohibition in force on Unknown);
//! - when the conditional path stored a re-checked deny head, every session agent a
//!   prohibition applies to is denied, since that head is consulted for every session.
#![cfg(feature = "odrl-bridge")]

use sparq_core::Graph;
use sparq_policy::{matched_prohibition, parse_policy_str, Request, ValidatedPolicy};
use sparq_solid::{Mode, PodStore, Session};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
const N1: &str = "https://pod.ex/notes/n1";
const PARTIES: [&str; 4] = [
    "https://alice.ex/card#me",
    "https://bob.ex/card#me",
    "https://carol.ex/card#me",
    "https://pod.ex/team",
];

const CONSTRAINTS: &[&str] = &[
    "",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ; odrl:rightOperand <https://alice.ex/card#me>",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ; odrl:rightOperand <https://alice.ex/card#me>",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:isAnyOf ; odrl:rightOperand \"https://alice.ex/card#me|https://bob.ex/card#me\"",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:isNoneOf ; odrl:rightOperand \"https://alice.ex/card#me\"",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ; odrl:rightOperand <https://pod.ex/team>",
    "odrl:leftOperand odrl:assignee ; odrl:operator odrl:eq ; odrl:rightOperand <https://alice.ex/card#me>",
    "odrl:leftOperand odrl:assignee ; odrl:operator odrl:neq ; odrl:rightOperand <https://alice.ex/card#me>",
    "odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ; odrl:rightOperand \"2030-01-01T00:00:00Z\"^^xsd:dateTime",
    "odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gteq ; odrl:rightOperand \"2020-01-01T00:00:00Z\"^^xsd:dateTime",
    "odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ; odrl:rightOperand \"2020-01-01T00:00:00Z\"^^xsd:dateTime",
    "odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/research>",
    "odrl:leftOperand odrl:spatial ; odrl:operator odrl:eq ; odrl:rightOperand <urn:geo/eu>",
    "odrl:leftOperand odrl:count ; odrl:operator odrl:lteq ; odrl:rightOperand 5",
    "odrl:leftOperand odrl:systemDevice ; odrl:operator odrl:eq ; odrl:rightOperand <urn:device/1>",
    "odrl:leftOperand <urn:unknown/operand> ; odrl:operator odrl:eq ; odrl:rightOperand 1",
    "a odrl:LogicalConstraint ; odrl:and ( [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ; odrl:rightOperand <https://alice.ex/card#me> ] )",
];

const ASSIGNEES: [Option<&str>; 3] =
    [None, Some("https://alice.ex/card#me"), Some("https://pod.ex/team")];

fn policy(constraint: &str, assignee: Option<&str>, collection: bool) -> Option<ValidatedPolicy> {
    let mut rule = format!("odrl:action odrl:read ; odrl:target <{N1}>");
    if let Some(a) = assignee {
        rule += &format!(" ; odrl:assignee <{a}>");
    }
    if !constraint.is_empty() {
        rule += &format!(" ; odrl:constraint [ {constraint} ]");
    }
    let mut ttl = format!(
        "@prefix odrl: <{ODRL}> .\n@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
         <urn:pol/p> a odrl:Set ; odrl:prohibition [ {rule} ] .\n"
    );
    if collection {
        ttl += "<https://pod.ex/team> a odrl:PartyCollection .\n";
    }
    parse_policy_str(&ttl, "turtle").ok()
}

/// Every session may read n1 through a static WAC rule.
fn public_store() -> PodStore {
    let nq = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#agentClass> <http://xmlns.com/foaf/0.1/Agent> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n1.acl> .
"#;
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").expect("pod loads"));
    store.materialize_wac().expect("wac");
    store
}

fn reads(store: &mut PodStore, agent: &str) -> bool {
    let s = Session { agent: Some(agent), client: None, issuer: None, now: None };
    store.accessible(&s, Mode::Read).iter().any(|g| g.as_str() == N1)
}

fn read_by(party: &str) -> Request {
    Request::new(format!("{ODRL}read")).on(N1).by(party)
}

fn check(store: &mut PodStore, pol: &ValidatedPolicy, req: &Request, head: bool, when: &str, case: &str) {
    let party = req.party.as_deref().unwrap();
    if matched_prohibition(pol, req).is_some() {
        assert!(!reads(store, party), "{when}: requester {party} not denied\n{case}");
    }
    if head {
        for s in PARTIES {
            if matched_prohibition(pol, &read_by(s)).is_some() {
                assert!(!reads(store, s), "{when}: session {s} not denied by the stored head\n{case}");
            }
        }
    }
}

#[test]
fn bridged_denies_cover_every_decide_deny() {
    let (mut cases, mut heads) = (0, 0);
    for constraint in CONSTRAINTS {
        for assignee in ASSIGNEES {
            for collection in [false, true] {
                let Some(pol) = policy(constraint, assignee, collection) else { continue };
                for party in PARTIES {
                    for at in [None, Some("2025-06-01T00:00:00Z")] {
                        let mut req = read_by(party);
                        if let Some(t) = at {
                            req = req.at(t);
                        }
                        for conditional in [false, true] {
                            let case = format!(
                                "constraint [{constraint}] assignee {assignee:?} collection \
                                 {collection} party {party} at {at:?} conditional {conditional}"
                            );
                            let mut store = public_store();
                            assert!(reads(&mut store, party), "public grant in force");
                            let out = if conditional {
                                store.materialize_odrl_prohibition_conditional(&pol, &req)
                            } else {
                                store.materialize_odrl_prohibition(&pol, &req)
                            };
                            let head = out.deny_triple.as_ref().is_some_and(|t| {
                                t.1 == "https://sparq.dev/ns/auth#effect"
                            });
                            heads += usize::from(head);
                            cases += 1;
                            check(&mut store, &pol, &req, head, "after materialize", &case);
                            store.refresh_odrl_grants();
                            check(&mut store, &pol, &req, head, "after refresh", &case);
                        }
                    }
                }
            }
        }
    }
    assert!(cases >= 500, "only {cases} cases ran");
    assert!(heads >= 20, "only {heads} conditional heads were stored");
}
