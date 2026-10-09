//! Request-time ODRL decisions (issue #6743) agree with `decide` for every session.
//!
//! Permissions and prohibitions are generated over every constraint left operand the
//! evaluator knows (and one it does not), compound constraints, assignees (a named
//! agent, one no request names, a party collection, wildcard principals) and targets
//! (an asset, an asset collection holding both assets, no target, an unrelated asset).
//! Each policy is attached to a store where n1 is publicly readable and n2 has no
//! grant. For every session (named parties, an unseen party, anonymous) and clock:
//!
//! - a graph is readable exactly when the static view or `decide` grants `odrl:read` and
//!   no prohibition applies to any read-family action (`matched_prohibition`, True or
//!   Unknown), through `accessible`, `query_as` and the point `decide`;
//! - the request-time layer never denies a session on n1 that the materialized
//!   conditional deny for the same policy lets through.
#![cfg(feature = "odrl-bridge")]

use sparq_core::Graph;
use sparq_policy::{decide, matched_prohibition, parse_policy_str, Request, ValidatedPolicy};
use sparq_solid::{Mode, PodStore, Session};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
const N1: &str = "https://pod.ex/notes/n1";
const N2: &str = "https://pod.ex/notes/n2";
const NOTES: &str = "https://pod.ex/notes/";
const ALICE: &str = "https://alice.ex/card#me";
const TEAM: &str = "https://pod.ex/team";
const UNSEEN: &str = "https://dave.ex/card#me";
const SESSIONS: [Option<&str>; 6] = [
    Some(ALICE),
    Some("https://bob.ex/card#me"),
    Some("https://carol.ex/card#me"),
    Some(TEAM),
    Some(UNSEEN),
    None,
];
const CLOCKS: [Option<&str>; 3] = [
    None,
    Some("2025-06-01T00:00:00Z"),
    Some("2035-06-01T00:00:00Z"),
];
const READ_ACTIONS: [&str; 5] = ["read", "display", "present", "print", "play"];

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

const ASSIGNEES: [Option<&str>; 6] = [
    None,
    Some(ALICE),
    Some(UNSEEN),
    Some(TEAM),
    Some("https://sparq.dev/ns/auth#Public"),
    Some("http://xmlns.com/foaf/0.1/Agent"),
];

const TARGETS: [Option<&str>; 4] = [Some(N1), Some(NOTES), None, Some("https://pod.ex/other")];

fn policy(
    kind: &str,
    action: &str,
    constraint: &str,
    assignee: Option<&str>,
    target: Option<&str>,
) -> Option<ValidatedPolicy> {
    let mut rule = format!("odrl:action odrl:{action}");
    if let Some(t) = target {
        rule += &format!(" ; odrl:target <{t}>");
    }
    if let Some(a) = assignee {
        rule += &format!(" ; odrl:assignee <{a}>");
    }
    if !constraint.is_empty() {
        rule += &format!(" ; odrl:constraint [ {constraint} ]");
    }
    let ttl = format!(
        "@prefix odrl: <{ODRL}> .\n@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
         <urn:pol/p> a odrl:Set ; odrl:{kind} [ {rule} ] .\n<{TEAM}> a odrl:PartyCollection .\n"
    );
    parse_policy_str(&ttl, "turtle").ok()
}

/// n1 is readable by every session through a static WAC rule; n2 has no grant.
fn bare_store() -> PodStore {
    let nq = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n2#it> <https://ex.dev/ns#title> "private" <https://pod.ex/notes/n2> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#agentClass> <http://xmlns.com/foaf/0.1/Agent> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n1.acl> .
"#;
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").expect("pod loads"));
    store.materialize_wac().expect("wac");
    store
}

/// [`bare_store`] with the evidence that n1 and n2 belong to the notes collection.
fn store() -> PodStore {
    let mut store = bare_store();
    store.add_odrl_asset_membership(N1, NOTES);
    store.add_odrl_asset_membership(N2, NOTES);
    store
}

fn session<'a>(agent: Option<&'a str>, now: Option<&'a str>) -> Session<'a> {
    Session {
        agent,
        client: None,
        issuer: None,
        now,
    }
}

fn reads(store: &PodStore, s: &Session, graph: &str) -> bool {
    store
        .accessible(s, Mode::Read)
        .iter()
        .any(|g| g.as_str() == graph)
}

fn request(action: &str, graph: &str, agent: Option<&str>, now: Option<&str>) -> Request {
    let mut req = Request::new(format!("{ODRL}{action}"))
        .on(graph)
        .with_asset_membership(N1, NOTES)
        .with_asset_membership(N2, NOTES);
    if let Some(a) = agent {
        req = req.by(a);
    }
    if let Some(t) = now {
        req = req.at(t);
    }
    req
}

/// What `decide` says about a read of `graph` by this session.
fn expected(pol: &ValidatedPolicy, graph: &str, agent: Option<&str>, now: Option<&str>) -> bool {
    let prohibited = READ_ACTIONS
        .iter()
        .any(|a| matched_prohibition(pol, &request(a, graph, agent, now)).is_some());
    let granted = decide(pol, &request("read", graph, agent, now)).allow;
    (graph == N1 || granted) && !prohibited
}

#[test]
fn request_time_decisions_match_decide_for_every_session() {
    let (mut cases, mut narrowed, mut granted) = (0, 0, 0);
    for kind in ["permission", "prohibition"] {
        for action in ["read", "use", "modify"] {
            for constraint in CONSTRAINTS {
                for assignee in ASSIGNEES {
                    for target in TARGETS {
                        let Some(pol) = policy(kind, action, constraint, assignee, target) else {
                            continue;
                        };
                        let mut live = store();
                        live.attach_odrl_policy(pol.clone()).expect("attach");
                        // The materialized conditional deny, as alice, for comparison.
                        let mut frozen = store();
                        if kind == "prohibition" {
                            frozen.materialize_odrl_prohibition_conditional(
                                &pol,
                                &request("read", N1, Some(ALICE), None),
                            );
                        }
                        for agent in SESSIONS {
                            for now in CLOCKS {
                                let s = session(agent, now);
                                let case = format!(
                                    "{kind} {action} [{constraint}] assignee {assignee:?} \
                                     target {target:?} session {agent:?} at {now:?}"
                                );
                                for graph in [N1, N2] {
                                    let want = expected(&pol, graph, agent, now);
                                    assert_eq!(
                                        reads(&live, &s, graph),
                                        want,
                                        "accessible {graph}: {case}"
                                    );
                                    let point = live.decide(&s, graph, Mode::Read);
                                    assert_eq!(point.allow, want, "point decide {graph}: {case}");
                                    if graph == N2 && want {
                                        granted += 1;
                                    }
                                    cases += 1;
                                }
                                let q = "SELECT ?t WHERE { GRAPH <https://pod.ex/notes/n1> { ?s ?p ?t } }";
                                let rows =
                                    live.query_as(&s, Mode::Read, q).expect("query").rows.len();
                                assert_eq!(
                                    rows == 1,
                                    expected(&pol, N1, agent, now),
                                    "query_as: {case}"
                                );
                                if kind == "prohibition" {
                                    let (l, f) = (reads(&live, &s, N1), reads(&frozen, &s, N1));
                                    assert!(
                                        l || !f,
                                        "request-time denies what the bridge allows: {case}"
                                    );
                                    narrowed += usize::from(l && !f);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(cases >= 20_000, "only {cases} cases ran");
    assert!(
        narrowed >= 500,
        "only {narrowed} sessions kept access the bridge over-denies"
    );
    assert!(granted >= 100, "only {granted} request-time grants on n2");
}

/// An asset that joins a prohibited collection is denied at once, with nothing
/// materialized for it, and a permission is re-checked against the session's clock.
#[test]
fn membership_and_clock_are_read_per_request() {
    let prohib = policy("prohibition", "read", "", None, Some(NOTES)).expect("policy");
    let mut s = bare_store();
    s.attach_odrl_policy(prohib).expect("attach");
    let anyone = session(Some(ALICE), None);
    assert!(
        reads(&s, &anyone, N1),
        "no membership evidence yet: n1 stays public"
    );
    s.add_odrl_asset_membership(N1, NOTES);
    assert!(
        !reads(&s, &anyone, N1),
        "n1 joined the collection: denied without materializing"
    );

    let window = "odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ; \
                  odrl:rightOperand \"2030-01-01T00:00:00Z\"^^xsd:dateTime";
    let permit = policy("permission", "read", window, Some(ALICE), Some(N2)).expect("policy");
    let mut s = store();
    s.attach_odrl_policy(permit).expect("attach");
    assert!(
        reads(&s, &session(Some(ALICE), Some("2025-06-01T00:00:00Z")), N2),
        "inside the window"
    );
    assert!(
        !reads(&s, &session(Some(ALICE), Some("2035-06-01T00:00:00Z")), N2),
        "after it"
    );
    assert!(
        !reads(&s, &session(Some(ALICE), None), N2),
        "no clock: fail closed"
    );
    assert!(!reads(
        &s,
        &session(Some("https://bob.ex/card#me"), Some("2025-06-01T00:00:00Z")),
        N2
    ));
}

/// A policy whose conflict strategy `decide` cannot honour is refused at attach time.
#[test]
fn unhonourable_conflict_strategy_is_refused() {
    let ttl = format!(
        "@prefix odrl: <{ODRL}> .\n<urn:pol/p> a odrl:Set ; odrl:conflict odrl:perm ; \
         odrl:permission [ odrl:action odrl:read ; odrl:target <{N2}> ] .\n"
    );
    let pol = parse_policy_str(&ttl, "turtle").expect("parses");
    assert!(store().attach_odrl_policy(pol).is_err());
}

/// Before the first materialization the view stays empty, even under an attached grant.
#[test]
fn an_unmaterialized_store_stays_closed() {
    let nq = "<https://pod.ex/notes/n2#it> <https://ex.dev/ns#title> \"x\" <https://pod.ex/notes/n2> .\n";
    let mut s = PodStore::new(Graph::load_dataset(nq, "nquads").expect("loads"));
    let permit = policy("permission", "read", "", None, Some(N2)).expect("policy");
    s.attach_odrl_policy(permit).expect("attach");
    let alice = session(Some(ALICE), None);
    assert!(!reads(&s, &alice, N2));
    assert!(!s.decide(&alice, N2, Mode::Read).allow);
    s.materialize_wac().expect("wac");
    assert!(reads(&s, &alice, N2), "granted once materialized");
    assert!(s.decide(&alice, N2, Mode::Read).allow);
}
