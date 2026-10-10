//! The bridge's denies cover every request `decide` denies.
//!
//! Prohibitions are generated over every constraint left operand the evaluator knows
//! (and one it does not), compound constraints, assignees (a named agent, one no
//! request names, a party collection, and wildcard principals), and targets (the asset,
//! an asset collection holding it, no target, and an unrelated asset). Each is layered
//! over a public WAC read grant through the one-shot and the conditional deny entry
//! points, materialized as one requesting party or anonymously. After materialization,
//! after a ledger refresh and after a WAC re-materialization that replays the ledger:
//!
//! - one-shot: the requester is denied whenever a prohibition still applies to its
//!   request (`matched_prohibition`, which keeps a prohibition in force on Unknown);
//! - conditional: every session `decide` denies is denied, whoever materialized. The
//!   session universe holds named parties, a party never seen at materialization, and
//!   an anonymous session.
#![cfg(feature = "odrl-bridge")]

use sparq_core::Graph;
use sparq_policy::{matched_prohibition, parse_policy_str, Request, ValidatedPolicy};
use sparq_solid::{Mode, PodStore, Session};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
const N1: &str = "https://pod.ex/notes/n1";
/// An asset collection holding n1, under every request's `odrl:partOf` evidence.
const NOTES: &str = "https://pod.ex/notes/";
const ALICE: &str = "https://alice.ex/card#me";
const TEAM: &str = "https://pod.ex/team";
/// A party no materializing request names.
const UNSEEN: &str = "https://dave.ex/card#me";
/// Session agents checked after materialization (`None` is anonymous).
const SESSIONS: [Option<&str>; 6] = [
    Some(ALICE),
    Some("https://bob.ex/card#me"),
    Some("https://carol.ex/card#me"),
    Some(TEAM),
    Some(UNSEEN),
    None,
];
/// Who materializes (`None` is an anonymous request).
const MATERIALIZERS: [Option<&str>; 2] = [Some(ALICE), None];

const CONSTRAINTS: &[&str] = &[
    "",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ; odrl:rightOperand <https://alice.ex/card#me>",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ; odrl:rightOperand <https://alice.ex/card#me>",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:isAnyOf ; odrl:rightOperand \"https://alice.ex/card#me|https://bob.ex/card#me\"",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:isNoneOf ; odrl:rightOperand \"https://alice.ex/card#me\"",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ; odrl:rightOperand <https://pod.ex/team>",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ; odrl:rightOperand <https://sparq.dev/ns/auth#Authenticated>",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ; odrl:rightOperand <https://sparq.dev/ns/auth#Public>",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:isNoneOf ; odrl:rightOperand \"https://sparq.dev/ns/auth#Authenticated\"",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ; odrl:rightOperand <https://sparq.dev/ns/auth#Public>",
    "odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ; odrl:rightOperand <http://xmlns.com/foaf/0.1/Agent>",
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

const ASSIGNEES: [Option<&str>; 8] = [
    None,
    Some(ALICE),
    Some(UNSEEN),
    Some(TEAM),
    Some("https://sparq.dev/ns/auth#Public"),
    Some("https://sparq.dev/ns/auth#Authenticated"),
    Some("http://xmlns.com/foaf/0.1/Agent"),
    Some("http://www.w3.org/ns/auth/acl#AuthenticatedAgent"),
];

const TARGETS: [Option<&str>; 4] = [Some(N1), Some(NOTES), None, Some("https://pod.ex/notes/n2")];

fn policy(
    constraint: &str,
    assignee: Option<&str>,
    collection: bool,
    target: Option<&str>,
) -> Option<ValidatedPolicy> {
    let mut rule = "odrl:action odrl:read".to_owned();
    if let Some(t) = target {
        rule += &format!(" ; odrl:target <{t}>");
    }
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
        ttl += &format!("<{TEAM}> a odrl:PartyCollection .\n");
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

fn reads(store: &mut PodStore, agent: Option<&str>) -> bool {
    let s = Session {
        agent,
        client: None,
        issuer: None,
        now: None,
    };
    store
        .accessible(&s, Mode::Read)
        .iter()
        .any(|g| g.as_str() == N1)
}

/// A read of n1 by `agent` (anonymous for `None`), carrying the `n1 odrl:partOf notes`
/// evidence.
fn read(agent: Option<&str>, at: Option<&str>) -> Request {
    let mut req = Request::new(format!("{ODRL}read"))
        .on(N1)
        .with_asset_membership(N1, NOTES);
    if let Some(a) = agent {
        req = req.by(a);
    }
    if let Some(t) = at {
        req = req.at(t);
    }
    req
}

/// Whether `decide` denies a read of n1 by `agent`, with or without a clock.
fn decide_denies(pol: &ValidatedPolicy, agent: Option<&str>) -> bool {
    [None, Some("2025-06-01T00:00:00Z")]
        .into_iter()
        .any(|at| matched_prohibition(pol, &read(agent, at)).is_some())
}

fn check(
    store: &mut PodStore,
    pol: &ValidatedPolicy,
    req: &Request,
    conditional: bool,
    when: &str,
    case: &str,
) {
    let party = req.party.as_deref();
    if matched_prohibition(pol, req).is_some() {
        assert!(
            !reads(store, party),
            "{when}: requester {party:?} not denied\n{case}"
        );
    }
    if conditional {
        for s in SESSIONS {
            if decide_denies(pol, s) {
                assert!(!reads(store, s), "{when}: session {s:?} not denied\n{case}");
            }
        }
    }
}

#[test]
fn bridged_denies_cover_every_decide_deny() {
    let (mut cases, mut denied) = (0, 0);
    for constraint in CONSTRAINTS {
        for assignee in ASSIGNEES {
            for collection in [false, true] {
                for target in TARGETS {
                    let Some(pol) = policy(constraint, assignee, collection, target) else {
                        continue;
                    };
                    if decide_denies(&pol, None) || SESSIONS.iter().any(|s| decide_denies(&pol, *s))
                    {
                        denied += 1;
                    }
                    for party in MATERIALIZERS {
                        for at in [None, Some("2025-06-01T00:00:00Z")] {
                            let req = read(party, at);
                            for conditional in [false, true] {
                                let case = format!(
                                    "constraint [{constraint}] assignee {assignee:?} collection \
                                     {collection} target {target:?} party {party:?} at {at:?} \
                                     conditional {conditional}"
                                );
                                let mut store = public_store();
                                assert!(reads(&mut store, party), "public grant in force");
                                if conditional {
                                    store.materialize_odrl_prohibition_conditional(&pol, &req);
                                } else {
                                    store.materialize_odrl_prohibition(&pol, &req);
                                }
                                cases += 1;
                                check(
                                    &mut store,
                                    &pol,
                                    &req,
                                    conditional,
                                    "after materialize",
                                    &case,
                                );
                                store.refresh_odrl_grants();
                                check(&mut store, &pol, &req, conditional, "after refresh", &case);
                                store.materialize_wac().expect("wac");
                                check(&mut store, &pol, &req, conditional, "after replay", &case);
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(cases >= 5000, "only {cases} cases ran");
    assert!(
        denied >= 300,
        "only {denied} policies deny some session under decide"
    );
}
