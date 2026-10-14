//! Request-time ODRL policies (issue #6743) agree with `decide` and
//! `matched_prohibition` for every session.
//!
//! Permissions and prohibitions are generated over every constraint left operand the
//! evaluator knows (and one it does not), compound constraints, assignees (a named
//! agent, one no request names, a party collection, wildcard principals) and targets
//! (an asset, an asset collection holding both assets, no target, an unrelated asset).
//! Each policy is attached to a store where n1 is publicly readable and n2 is readable
//! by alice alone. For every session (named parties, an unseen party, anonymous) and
//! clock:
//!
//! - a graph is readable exactly when the static view grants it or `decide` permits
//!   `odrl:read` for the session under every party membership it could have, and no
//!   prohibition applies to any read-family action (True or Unknown), through
//!   `accessible`, `query_as` and the point `decide`;
//! - the request-time layer never denies a session on n1 that the materialized
//!   conditional deny for the same policy lets through.
//!
//! The remaining tests pin the mutation paths, the graphs the layer never touches, the
//! `WAC-Allow` clock, and an enumeration of every entry point that takes a session.
#![cfg(feature = "odrl-bridge")]

use oxrdf::NamedNode;
use sparq_core::Graph;
use sparq_engine::QueryBudget;
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

/// n1 is readable by every session through its own ACL; alice holds every mode on
/// everything else (n2 and the notes container) through the root ACL.
fn bare_store() -> PodStore {
    bare_store_with("")
}

/// [`bare_store`] plus the N-Quads `extra`.
fn bare_store_with(extra: &str) -> PodStore {
    let nq = r#"
<https://pod.ex/.acl#alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Append> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Control> <https://pod.ex/.acl> .
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n2#it> <https://ex.dev/ns#title> "private" <https://pod.ex/notes/n2> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#agentClass> <http://xmlns.com/foaf/0.1/Agent> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n1.acl> .
"#;
    let mut store =
        PodStore::new(Graph::load_dataset(&format!("{nq}{extra}"), "nquads").expect("pod loads"));
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

/// Whether a prohibition applies to `req` for an agent whose membership of the policy's
/// party collections is unknown: as a non-member, or as a member of any of them.
fn applies(pol: &ValidatedPolicy, req: Request, agent: Option<&str>) -> bool {
    if matched_prohibition(pol, &req).is_some() {
        return true;
    }
    let Some(agent) = agent else { return false };
    pol.party_collections.iter().any(|c| {
        matched_prohibition(pol, &req.clone().with_party_membership(agent, c.as_str())).is_some()
    })
}

/// Whether `decide` permits `req` whatever the agent's membership of the policy's party
/// collections: as a non-member, and as a member of each subset of them.
fn permits(pol: &ValidatedPolicy, req: Request, agent: Option<&str>) -> bool {
    let collections: Vec<&String> = pol.party_collections.iter().collect();
    if !decide(pol, &req).allow {
        return false;
    }
    let Some(agent) = agent else { return true };
    (1u32..1 << collections.len()).all(|mask| {
        let mut r = req.clone();
        for (i, c) in collections.iter().enumerate() {
            if mask & (1 << i) != 0 {
                r = r.with_party_membership(agent, c.as_str());
            }
        }
        decide(pol, &r).allow
    })
}

/// The static read verdict, widened by a permitted `odrl:read` and narrowed by every
/// read-family prohibition that applies.
fn expected(pol: &ValidatedPolicy, graph: &str, agent: Option<&str>, now: Option<&str>) -> bool {
    let prohibited = READ_ACTIONS
        .iter()
        .any(|a| applies(pol, request(a, graph, agent, now), agent));
    let static_read = graph == N1 || agent == Some(ALICE);
    let permitted = permits(pol, request("read", graph, agent, now), agent);
    (static_read || permitted) && !prohibited
}

#[test]
fn request_time_decisions_match_decide_for_every_session() {
    let (mut cases, mut narrowed, mut denied, mut widened) = (0, 0, 0, 0);
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
                                    assert_eq!(
                                        point.granted_modes.contains(&Mode::Read),
                                        want,
                                        "granted modes {graph}: {case}"
                                    );
                                    let static_read = graph == N1 || agent == Some(ALICE);
                                    denied += usize::from(static_read && !want);
                                    widened += usize::from(!static_read && want);
                                    cases += 1;
                                }
                                for graph in [N1, N2] {
                                    let q = format!("SELECT ?t WHERE {{ GRAPH <{graph}> {{ ?s ?p ?t }} }}");
                                    let rows =
                                        live.query_as(&s, Mode::Read, &q).expect("query").rows.len();
                                    assert_eq!(
                                        rows == 1,
                                        expected(&pol, graph, agent, now),
                                        "query_as {graph}: {case}"
                                    );
                                }
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
    assert!(denied >= 1_000, "only {denied} static grants were narrowed");
    assert!(widened >= 500, "only {widened} permissions granted beyond the static view");
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
    let prohib = policy("prohibition", "read", window, Some(ALICE), Some(N2)).expect("policy");
    let mut s = store();
    s.attach_odrl_policy(prohib).expect("attach");
    assert!(
        !reads(&s, &session(Some(ALICE), Some("2025-06-01T00:00:00Z")), N2),
        "inside the window"
    );
    assert!(
        reads(&s, &session(Some(ALICE), Some("2035-06-01T00:00:00Z")), N2),
        "after it"
    );
    assert!(
        !reads(&s, &session(Some(ALICE), None), N2),
        "no clock: the constraint is Unknown, so the prohibition applies"
    );
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

/// Party-collection membership is enumerated per request, so a policy with
/// prohibitions may name at most four collections; a larger one is refused rather than
/// widened to deny outside its scope. Permission-only policies are unaffected.
#[test]
fn too_many_party_collections_are_refused() {
    let with = |n: usize, kind: &str| {
        let teams: String = (0..n)
            .map(|i| format!("<https://pod.ex/team{i}> a odrl:PartyCollection .\n"))
            .collect();
        let ttl = format!(
            "@prefix odrl: <{ODRL}> .\n<urn:pol/p> a odrl:Set ; odrl:{kind} \
             [ odrl:action odrl:append ; odrl:target <{N2}> ] .\n{teams}"
        );
        parse_policy_str(&ttl, "turtle").expect("parses")
    };
    let mut s = store();
    assert!(s.attach_odrl_policy(with(5, "prohibition")).is_err());
    assert!(reads(&s, &session(Some(ALICE), None), N1), "nothing attached");
    s.attach_odrl_policy(with(5, "permission")).expect("permission-only");
    s.attach_odrl_policy(with(4, "prohibition")).expect("four collections");
    let alice = session(Some(ALICE), None);
    assert!(reads(&s, &alice, N1) && reads(&s, &alice, N2), "append-only prohibition");
    assert!(!s.decide(&alice, N2, Mode::Append).allow);
}

const BOB: &str = "https://bob.ex/card#me";

/// An attached `odrl:read` permission grants `Read` where `decide` permits it, through
/// every entry point, and nothing else: no other mode, no access-control document, no
/// reserved graph.
#[test]
fn permissions_grant_read_through_every_entry_point() {
    let mut s = store();
    s.attach_odrl_policy(policy("permission", "read", "", None, None).expect("policy"))
        .expect("attach");
    let bob = session(Some(BOB), None);
    let before = bare_store();
    assert!(!before.decide(&bob, N2, Mode::Read).allow, "no static grant");
    let d = s.decide(&bob, N2, Mode::Read);
    assert!(d.allow && d.granted_modes == vec![Mode::Read]);
    assert!(reads(&s, &bob, N2));
    let rows = s
        .query_as(&bob, Mode::Read, "SELECT ?t WHERE { GRAPH <https://pod.ex/notes/n2> { ?s ?p ?t } }")
        .expect("query")
        .rows
        .len();
    assert_eq!(rows, 1);
    for mode in [Mode::Write, Mode::Append, Mode::Control] {
        assert_eq!(s.accessible(&bob, mode), before.accessible(&bob, mode), "{mode:?}");
        assert!(!s.decide(&bob, N2, mode).allow, "{mode:?}");
        assert!(!s.decide_create(&bob, NOTES, "n3", mode).allow, "{mode:?}");
    }
    for graph in ["https://pod.ex/notes/n1.acl", "https://pod.ex/.acl", "urn:sparq:auth"] {
        assert!(!s.decide(&bob, graph, Mode::Read).allow, "{graph}");
    }
    let ins = "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 1 } }";
    assert!(s.update_as(&bob, ins).is_err());
}

const CHANGE_ACTIONS: [&str; 4] = ["append", "modify", "delete", "write"];
/// The notes container as a graph of its own.
const CONTAINER: &str = "<https://pod.ex/notes/> <http://www.w3.org/ns/ldp#contains> \
                         <https://pod.ex/notes/n2> <https://pod.ex/notes/> .\n";

/// One policy whose permissions name `odrl:read`, `modify`, `delete` and `append`: it
/// grants Read, Write and Append wherever its rules apply.
fn change_policy() -> ValidatedPolicy {
    let ttl = format!(
        "@prefix odrl: <{ODRL}> .\n<urn:pol/p> a odrl:Set ; odrl:permission \
         [ odrl:action odrl:read ] , [ odrl:action odrl:modify ] , \
         [ odrl:action odrl:delete ] , [ odrl:action odrl:append ] .\n"
    );
    parse_policy_str(&ttl, "turtle").expect("parses")
}

/// Whether `session` holds `mode` on `graph`, asserting that `decide`, `decide_batch`,
/// the granted modes and `accessible` agree.
fn holds(s: &PodStore, session: &Session, graph: &str, mode: Mode) -> bool {
    let point = s.decide(session, graph, mode);
    let batch = s.decide_batch(session, &[(graph, mode)]);
    let set = s.accessible(session, mode).iter().any(|g| g.as_str() == graph);
    assert_eq!(point.allow, set, "decide vs accessible {graph} {mode:?}");
    assert_eq!(point.allow, batch[0].allow, "decide vs decide_batch {graph} {mode:?}");
    assert_eq!(point.allow, point.granted_modes.contains(&mode), "granted modes {graph} {mode:?}");
    point.allow
}

/// `Write` and `Append` follow `decide` for every session: `Write` needs one policy to
/// permit both `odrl:modify` and `odrl:delete`, `Append` needs `odrl:append` or that
/// `Write` condition, each through a rule naming the action (so `odrl:use` grants
/// neither), and any change-family prohibition removes both.
#[test]
fn change_grants_match_decide_for_every_session() {
    let constraints =
        [CONSTRAINTS[0], CONSTRAINTS[1], CONSTRAINTS[11], CONSTRAINTS[13], CONSTRAINTS[16]];
    let (mut cases, mut widened, mut narrowed) = (0, 0, 0);
    for kind in ["permission", "prohibition"] {
        for action in ["use", "modify", "delete", "append", "read"] {
            for constraint in constraints {
                for assignee in ASSIGNEES {
                    for target in [Some(N2), Some(NOTES), None] {
                        let Some(pol) = policy(kind, action, constraint, assignee, target) else {
                            continue;
                        };
                        // The container exists as a graph, so only the container rule
                        // keeps a change grant off it.
                        let mut live = bare_store_with(CONTAINER);
                        live.add_odrl_asset_membership(N1, NOTES);
                        live.add_odrl_asset_membership(N2, NOTES);
                        live.attach_odrl_policy(pol.clone()).expect("attach");
                        for agent in SESSIONS {
                            for now in CLOCKS {
                                let s = session(agent, now);
                                let prohibited = CHANGE_ACTIONS
                                    .iter()
                                    .any(|a| applies(&pol, request(a, N2, agent, now), agent));
                                // Change modes come only from a rule naming the action.
                                let permit = |a: &str| {
                                    action == a
                                        && permits(&pol, request(a, N2, agent, now), agent)
                                };
                                let writes = permit("modify") && permit("delete");
                                let static_change = agent == Some(ALICE);
                                let want_write = (static_change || writes) && !prohibited;
                                let want_append =
                                    (static_change || writes || permit("append")) && !prohibited;
                                let case = format!(
                                    "{kind} {action} [{constraint}] assignee {assignee:?} \
                                     target {target:?} session {agent:?} at {now:?}"
                                );
                                let write = holds(&live, &s, N2, Mode::Write);
                                let append = holds(&live, &s, N2, Mode::Append);
                                assert_eq!(write, want_write, "write: {case}");
                                assert_eq!(append, want_append, "append: {case}");
                                // The container is never granted a change mode.
                                assert_eq!(
                                    live.decide(&s, NOTES, Mode::Append).allow,
                                    static_change && !prohibited_on(&pol, NOTES, agent, now),
                                    "container: {case}"
                                );
                                widened += usize::from(!static_change && want_append);
                                narrowed += usize::from(static_change && !want_write);
                                cases += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(cases >= 15_000, "only {cases} cases ran");
    assert!(widened >= 70, "only {widened} change grants beyond the static view");
    assert!(narrowed >= 450, "only {narrowed} static change grants were narrowed");
}

/// Whether a change-family prohibition of `pol` applies on `graph`.
fn prohibited_on(
    pol: &ValidatedPolicy,
    graph: &str,
    agent: Option<&str>,
    now: Option<&str>,
) -> bool {
    CHANGE_ACTIONS
        .iter()
        .any(|a| applies(pol, request(a, graph, agent, now), agent))
}

/// A permission naming the change actions grants `Write` and `Append` on an existing
/// resource, through the
/// point decisions, the session sets and every update path, but never on the container,
/// never a create, never `Control` and never an access-control document.
#[test]
fn change_permissions_grant_existing_resources_only() {
    // The container exists as a graph of its own, so only the container rule keeps it out.
    let container = CONTAINER;
    let mut s = bare_store_with(container);
    s.attach_odrl_policy(change_policy()).expect("attach");
    let bob = session(Some(BOB), None);
    assert!(holds(&s, &bob, NOTES, Mode::Read), "the container is read-granted");
    let before = bare_store_with(container);
    let d = s.decide(&bob, N2, Mode::Write);
    assert!(d.allow && d.granted_modes == vec![Mode::Read, Mode::Write, Mode::Append]);
    for mode in [Mode::Write, Mode::Append] {
        assert!(holds(&s, &bob, N2, mode), "{mode:?}");
        assert!(!holds(&s, &bob, NOTES, mode), "container {mode:?}");
        assert!(!s.decide_create(&bob, NOTES, "n3", mode).allow, "create {mode:?}");
    }
    assert_eq!(s.accessible(&bob, Mode::Control), before.accessible(&bob, Mode::Control));
    assert!(!s.decide(&bob, N2, Mode::Control).allow);
    let acls = [
        "https://pod.ex/.acl",
        "https://pod.ex/notes/n1.acl",
        "https://pod.ex/notes/n2.acl",
    ];
    for acl in acls {
        assert!(!s.decide(&bob, acl, Mode::Write).allow, "{acl}");
        let ins = format!("INSERT DATA {{ GRAPH <{acl}> {{ <urn:x> <urn:y> 1 }} }}");
        assert!(s.update_as(&bob, &ins).is_err(), "{acl}");
    }
    s.update_as(&bob, "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 1 } }")
        .expect("granted Append/Write inserts");
    s.update_as(&bob, "DELETE DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 1 } }")
        .expect("granted Write deletes");
    s.update_as(
        &bob,
        "DELETE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } } \
         INSERT { GRAPH ?g { ?s <https://ex.dev/ns#title> \"renamed\" } } \
         WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }",
    )
    .expect("granted Write rewrites through GRAPH ?g");
    let contain = "INSERT DATA { GRAPH <https://pod.ex/notes/> { <https://pod.ex/notes/> \
                   <http://www.w3.org/ns/ldp#contains> <https://pod.ex/notes/n3> } }";
    assert!(s.update_as(&bob, contain).is_err(), "no write to the container");
    let fresh_graphs = [
        "https://pod.ex/notes/n3",
        "https://pod.ex/notes/sub/",
        "https://pod.ex/elsewhere",
    ];
    for fresh in fresh_graphs {
        let ins = format!("INSERT DATA {{ GRAPH <{fresh}> {{ <urn:x> <urn:y> 1 }} }}");
        assert!(s.update_as(&bob, &ins).is_err(), "no create of {fresh}");
    }
    // Graph management rests on the static view: no grant applies in a request that drops,
    // clears, creates, loads, copies or moves a graph.
    let managed = [
        "DROP GRAPH <https://pod.ex/notes/n2>",
        "CLEAR GRAPH <https://pod.ex/notes/n2>",
        "DROP GRAPH <https://pod.ex/notes/n2> ; \
         INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 2 } }",
        "DROP GRAPH <https://pod.ex/notes/n2> ; CREATE GRAPH <https://pod.ex/notes/n2>",
        "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 3 } } ; \
         DROP GRAPH <https://pod.ex/notes/n2>",
        "COPY <https://pod.ex/notes/n1> TO <https://pod.ex/notes/n2>",
        "MOVE <https://pod.ex/notes/n1> TO <https://pod.ex/notes/n2>",
    ];
    for upd in managed {
        assert!(s.update_as(&bob, upd).is_err(), "{upd}");
    }
    let alice = session(Some(ALICE), None);
    let mut owner = bare_store_with(container);
    let recreate = managed[2];
    owner.update_as(&alice, recreate).expect("the static grant re-creates");
    owner
        .attach_odrl_policy(
            policy("prohibition", "modify", "", Some(ALICE), Some(N2)).expect("policy"),
        )
        .expect("attach");
    assert!(owner.update_as(&alice, recreate).is_err(), "a prohibition still applies");
}

/// `odrl:use` permits every action under `decide`, but grants `Read` only here: the
/// change modes need a permission that names the change action.
#[test]
fn use_grants_read_only() {
    let mut s = store();
    s.attach_odrl_policy(policy("permission", "use", "", None, None).expect("policy"))
        .expect("attach");
    let bob = session(Some(BOB), None);
    assert!(holds(&s, &bob, N2, Mode::Read));
    assert!(!holds(&s, &bob, N2, Mode::Write) && !holds(&s, &bob, N2, Mode::Append));
    let ins = "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 1 } }";
    assert!(s.update_as(&bob, ins).is_err());
    // A `use` rule beside a narrower named one does not widen it: modify for alice only.
    let ttl = format!(
        "@prefix odrl: <{ODRL}> .\n<urn:pol/p> a odrl:Set ; odrl:permission \
         [ odrl:action odrl:use ] , \
         [ odrl:action odrl:modify ; odrl:assignee <{ALICE}> ] , \
         [ odrl:action odrl:delete ; odrl:assignee <{ALICE}> ] .\n"
    );
    let mut mixed = store();
    mixed.attach_odrl_policy(parse_policy_str(&ttl, "turtle").expect("parses")).expect("attach");
    assert!(!holds(&mixed, &bob, N2, Mode::Write), "use does not stand in for modify");
}

/// A static writer of every graph may write a graph, drop it, then `DROP NAMED`, with a
/// policy attached: the wildcard checks only the graphs still there.
#[test]
fn drop_then_wildcard_stays_open_to_a_static_writer() {
    let nq = r#"
<https://pod.ex/.acl#alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/.acl> .
<https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Control> <https://pod.ex/.acl> .
<https://pod.ex/notes/n2#it> <https://ex.dev/ns#title> "private" <https://pod.ex/notes/n2> .
<https://pod.ex/notes/n3#it> <https://ex.dev/ns#title> "other" <https://pod.ex/notes/n3> .
"#;
    let mut s = PodStore::new(Graph::load_dataset(nq, "nquads").expect("pod loads"));
    s.materialize_wac().expect("wac");
    s.attach_odrl_policy(change_policy()).expect("attach");
    let wipe = "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 1 } } ; \
                DROP GRAPH <https://pod.ex/notes/n2> ; DROP NAMED";
    s.update_as(&session(Some(ALICE), None), wipe).expect("alice may drop every graph");
}

/// In a request that also manages a graph, a grant authorizes none of its writes, while a
/// data-only request combines grants with the static view.
#[test]
fn graph_management_requests_use_static_grants_only() {
    let bob_writes_n3 = format!(
        "{CONTAINER}\
<https://pod.ex/notes/n3.acl#bob> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n3.acl> .
<https://pod.ex/notes/n3.acl#bob> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n3> <https://pod.ex/notes/n3.acl> .
<https://pod.ex/notes/n3#it> <https://ex.dev/ns#title> \"mine\" <https://pod.ex/notes/n3> .
<https://pod.ex/notes/n3.acl#bob> <http://www.w3.org/ns/auth/acl#agent> <{BOB}> <https://pod.ex/notes/n3.acl> .
<https://pod.ex/notes/n3.acl#bob> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n3.acl> .
<https://pod.ex/notes/n3.acl#bob> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/notes/n3.acl> .
"
    );
    let bob = session(Some(BOB), None);
    let mut s = bare_store_with(&bob_writes_n3);
    s.attach_odrl_policy(change_policy()).expect("attach");
    assert!(holds(&s, &bob, N2, Mode::Write), "Write on n2 through the grant");
    let data = "DELETE DATA { GRAPH <https://pod.ex/notes/n2> { <https://pod.ex/notes/n2#it> \
                <https://ex.dev/ns#title> \"private\" } } ; \
                INSERT DATA { GRAPH <https://pod.ex/notes/n3> { <urn:x> <urn:y> 1 } }";
    s.update_as(&bob, data).expect("grant and static write combine in a data-only request");
    s.update_as(&bob, "ADD <https://pod.ex/notes/n3> TO <https://pod.ex/notes/n2>")
        .expect("ADD is a data change into an existing granted graph");
    let add_fresh = "ADD <https://pod.ex/notes/n3> TO <https://pod.ex/notes/n9>";
    assert!(s.update_as(&bob, add_fresh).is_err(), "ADD creates no graph through a grant");
    let mixed = [
        "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 2 } } ; \
         DROP GRAPH <https://pod.ex/notes/n3>",
        "CLEAR GRAPH <https://pod.ex/notes/n3> ; \
         INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 2 } }",
        "CREATE SILENT GRAPH <https://pod.ex/notes/n3> ; \
         INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 2 } }",
    ];
    for upd in mixed {
        assert!(s.update_as(&bob, upd).is_err(), "{upd}");
    }
    assert!(s.update_as(&bob, "DROP NAMED").is_err(), "no wildcard through a grant");
    s.update_as(&bob, "CLEAR GRAPH <https://pod.ex/notes/n3> ; DROP GRAPH <https://pod.ex/notes/n3>")
        .expect("static Write manages n3");
    s.update_as(&bob, "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 4 } }")
        .expect("the grant still applies to a later data-only request");
    assert!(s.decide(&bob, N2, Mode::Write).allow, "a data change through the grant");
    assert!(!s.decide_removal(&bob, N2).allow, "no removal through the grant");
    assert!(s.decide_removal(&bob, "https://pod.ex/notes/n3").allow, "static Write removes");
    let alice = session(Some(ALICE), None);
    assert!(s.decide_removal(&alice, N2).allow);
    s.attach_odrl_policy(policy("prohibition", "delete", "", Some(ALICE), Some(N2)).expect("policy"))
        .expect("attach");
    assert!(!s.decide_removal(&alice, N2).allow, "a prohibition still applies");
}

/// `Write` needs `odrl:modify` and `odrl:delete` from one policy; `Append` alone never
/// implies `Write`.
#[test]
fn write_needs_modify_and_delete_in_one_policy() {
    let bob = session(Some(BOB), None);
    let mut modify = store();
    modify.attach_odrl_policy(policy("permission", "modify", "", None, None).expect("policy"))
        .expect("attach");
    assert!(!holds(&modify, &bob, N2, Mode::Write) && !holds(&modify, &bob, N2, Mode::Append));
    modify.attach_odrl_policy(policy("permission", "delete", "", None, None).expect("policy"))
        .expect("attach");
    assert!(!holds(&modify, &bob, N2, Mode::Write), "split across policies");

    // Both rules in one policy: Write, and Append through it with no `odrl:append`.
    let ttl = format!(
        "@prefix odrl: <{ODRL}> .\n<urn:pol/p> a odrl:Set ; \
         odrl:permission [ odrl:action odrl:modify ] , [ odrl:action odrl:delete ] .\n"
    );
    let mut both = store();
    both.attach_odrl_policy(parse_policy_str(&ttl, "turtle").expect("parses"))
        .expect("attach");
    assert!(holds(&both, &bob, N2, Mode::Write) && holds(&both, &bob, N2, Mode::Append));

    let mut append = store();
    append.attach_odrl_policy(policy("permission", "append", "", None, None).expect("policy"))
        .expect("attach");
    assert!(holds(&append, &bob, N2, Mode::Append) && !holds(&append, &bob, N2, Mode::Write));
    append
        .update_as(&bob, "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 1 } }")
        .expect("Append inserts");
    let del = "DELETE DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 1 } }";
    assert!(append.update_as(&bob, del).is_err(), "Append does not delete");
}

/// A referenced group document is never granted a change mode: a write there would
/// change who holds the static grants.
#[test]
fn group_documents_are_never_granted_a_change() {
    const GROUPS: &str = "https://pod.ex/groups";
    let extra = r#"
<https://pod.ex/groups#team> <http://www.w3.org/2006/vcard/ns#hasMember> <https://carol.ex/card#me> <https://pod.ex/groups> .
<https://pod.ex/.acl#team> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#agentGroup> <https://pod.ex/groups#team> <https://pod.ex/.acl> .
<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
"#;
    let mut s = bare_store_with(extra);
    s.attach_odrl_policy(change_policy()).expect("attach");
    let bob = session(Some(BOB), None);
    assert!(holds(&s, &bob, GROUPS, Mode::Read), "Read is still granted");
    for mode in [Mode::Write, Mode::Append] {
        assert!(!holds(&s, &bob, GROUPS, mode), "{mode:?}");
    }
    let join = format!(
        "INSERT DATA {{ GRAPH <{GROUPS}> {{ <https://pod.ex/groups#team> \
         <http://www.w3.org/2006/vcard/ns#hasMember> <{BOB}> }} }}"
    );
    assert!(s.update_as(&bob, &join).is_err(), "bob cannot join the group");
    assert!(holds(&s, &bob, N2, Mode::Write), "an ordinary resource is still granted");
}

/// An explicit ACP deny of `Write` also stops the `Append` a `Write` grant carries; an
/// `odrl:append` permission still grants `Append`, which the deny does not cover.
#[test]
fn acp_write_deny_also_stops_append_through_write() {
    const ACP: &str = "http://www.w3.org/ns/solid/acp#";
    const ACL: &str = "http://www.w3.org/ns/auth/acl#";
    let g = "https://pod.ex/.acr";
    let mut nq = String::from(
        "<https://pod.ex/notes/n2#it> <https://ex.dev/ns#title> \"private\" <https://pod.ex/notes/n2> .\n",
    );
    for (s_, p, o) in [
        (g, "resource", "https://pod.ex/"),
        (g, "memberAccessControl", "https://pod.ex/.acr#c"),
        ("https://pod.ex/.acr#c", "apply", "https://pod.ex/.acr#p"),
        ("https://pod.ex/.acr#p", "deny", "WRITE"),
        ("https://pod.ex/.acr#p", "anyOf", "https://pod.ex/.acr#m"),
        ("https://pod.ex/.acr#m", "agent", BOB),
    ] {
        let o = if o == "WRITE" { format!("{ACL}Write") } else { o.to_owned() };
        nq += &format!("<{s_}> <{ACP}{p}> <{o}> <{g}> .\n");
    }
    let acp_store = || {
        let mut acp = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("loads"));
        acp.materialize_acp().expect("acp");
        acp
    };
    let ttl = format!(
        "@prefix odrl: <{ODRL}> .\n<urn:pol/p> a odrl:Set ; \
         odrl:permission [ odrl:action odrl:modify ] , [ odrl:action odrl:delete ] .\n"
    );
    let bob = session(Some(BOB), None);
    let carol = session(Some("https://carol.ex/card#me"), None);
    let mut acp = acp_store();
    acp.attach_odrl_policy(parse_policy_str(&ttl, "turtle").expect("parses")).expect("attach");
    assert!(holds(&acp, &carol, N2, Mode::Write) && holds(&acp, &carol, N2, Mode::Append));
    assert!(!holds(&acp, &bob, N2, Mode::Write), "bob's Write deny wins");
    assert!(!holds(&acp, &bob, N2, Mode::Append), "and stops the Append that Write carries");
    let ins = "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 1 } }";
    assert!(acp.update_as_acp(&bob, ins).is_err());

    acp.attach_odrl_policy(policy("permission", "append", "", None, None).expect("policy"))
        .expect("attach");
    assert!(holds(&acp, &bob, N2, Mode::Append), "an append permission is not denied");
    assert!(!holds(&acp, &bob, N2, Mode::Write));
    acp.update_as_acp(&bob, ins).expect("bob appends");
}

/// A `GRAPH ?g` update reaching a referenced group document, or a container, is refused
/// to a grantee; one name whose last segment reads as a control document in another case
/// is granted `Read` only.
#[test]
fn change_grants_hold_through_variable_graphs_and_odd_names() {
    let extra = format!(
        "{CONTAINER}\
<https://pod.ex/groups#team> <http://www.w3.org/2006/vcard/ns#hasMember> <https://carol.ex/card#me> <https://pod.ex/groups> .
<https://pod.ex/.acl#team> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#agentGroup> <https://pod.ex/groups#team> <https://pod.ex/.acl> .
<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
<https://pod.ex/notes/r.ACL#it> <https://ex.dev/ns#title> \"odd\" <https://pod.ex/notes/r.ACL> .
"
    );
    let mut s = bare_store_with(&extra);
    s.attach_odrl_policy(change_policy()).expect("attach");
    let bob = session(Some(BOB), None);
    for target in ["https://pod.ex/groups", NOTES] {
        let upd = format!(
            "INSERT {{ GRAPH ?g {{ <urn:x> <urn:y> 1 }} }} WHERE {{ VALUES ?g {{ <{target}> }} }}"
        );
        assert!(s.update_as(&bob, &upd).is_err(), "{target}");
    }
    let odd = "https://pod.ex/notes/r.ACL";
    assert!(holds(&s, &bob, odd, Mode::Read));
    assert!(!holds(&s, &bob, odd, Mode::Write) && !holds(&s, &bob, odd, Mode::Append));
    let upd = "INSERT { GRAPH ?g { <urn:x> <urn:y> 1 } } \
               WHERE { VALUES ?g { <https://pod.ex/notes/n2> } }";
    s.update_as(&bob, upd).expect("an ordinary resource is still granted");
}

/// A grant needs a graph that exists now and that the last static materialization
/// covered: one created since, or one dropped since, is not granted.
#[test]
fn grants_need_a_current_covered_graph() {
    let mut s = store();
    s.attach_odrl_policy(policy("permission", "read", "", None, None).expect("policy"))
        .expect("attach");
    let alice = session(Some(ALICE), None);
    let bob = session(Some(BOB), None);
    let fresh = "https://pod.ex/notes/n9";
    assert!(!s.decide(&bob, fresh, Mode::Read).allow, "does not exist");
    // A graph added to the dataset without re-materializing.
    let added = Graph::load_dataset(
        &format!("<urn:x> <urn:y> \"1\" <{fresh}> ."),
        "nquads",
    )
    .expect("loads");
    s.graph.named.extend(added.named);
    assert!(!s.decide(&bob, fresh, Mode::Read).allow, "created since the materialization");
    assert!(!reads(&s, &bob, fresh));
    s.materialize_wac().expect("wac");
    assert!(s.decide(&bob, fresh, Mode::Read).allow, "covered once materialized");
    assert!(reads(&s, &bob, N2));
    s.update_as(&alice, "DROP GRAPH <https://pod.ex/notes/n2>").expect("alice drops");
    assert!(!s.decide(&bob, N2, Mode::Read).allow, "dropped since");
    // Removed, then created again under the same name: not covered until re-materialized,
    // whether the two happen in separate updates or within one.
    s.update_as(&alice, "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:a> <urn:b> \"2\" } }")
        .expect("alice recreates");
    assert!(!s.decide(&bob, N2, Mode::Read).allow, "recreated by a later update");
    assert!(!reads(&s, &bob, N2));
    assert!(s.decide(&bob, fresh, Mode::Read).allow);
    s.update_as(
        &alice,
        &format!("DROP GRAPH <{fresh}> ; INSERT DATA {{ GRAPH <{fresh}> {{ <urn:a> <urn:b> \"3\" }} }}"),
    )
    .expect("alice replaces in one update");
    assert!(!s.decide(&bob, fresh, Mode::Read).allow, "recreated within one update");
    assert!(!reads(&s, &bob, fresh));
    s.materialize_wac().expect("wac");
    assert!(s.decide(&bob, N2, Mode::Read).allow && s.decide(&bob, fresh, Mode::Read).allow);
}

/// Time-windowed and recipient-scoped permissions follow the accessing session.
#[test]
fn windows_and_recipients_follow_the_session() {
    let window = "odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ; \
                  odrl:rightOperand \"2030-01-01T00:00:00Z\"^^xsd:dateTime";
    let mut s = store();
    s.attach_odrl_policy(policy("permission", "read", window, None, Some(N2)).expect("policy"))
        .expect("attach");
    assert!(reads(&s, &session(Some(BOB), Some("2025-06-01T00:00:00Z")), N2));
    assert!(!reads(&s, &session(Some(BOB), Some("2035-06-01T00:00:00Z")), N2));
    assert!(!reads(&s, &session(Some(BOB), None), N2), "no clock");

    let except_bob = "odrl:leftOperand odrl:recipient ; odrl:operator odrl:isNoneOf ; \
                      odrl:rightOperand \"https://bob.ex/card#me\"";
    let mut s = store();
    s.attach_odrl_policy(policy("permission", "read", except_bob, None, Some(N2)).expect("policy"))
        .expect("attach");
    assert!(reads(&s, &session(Some("https://carol.ex/card#me"), None), N2));
    assert!(!reads(&s, &session(Some(BOB), None), N2));
    assert!(!reads(&s, &session(None, None), N2), "anonymous has no recipient");
}

/// A permission never overrides an explicit deny of the auth view, and never grants on
/// a graph no access-control document governs.
#[test]
fn explicit_denies_and_ungoverned_graphs_win() {
    const ACP: &str = "http://www.w3.org/ns/solid/acp#";
    const ACL: &str = "http://www.w3.org/ns/auth/acl#";
    let g = "https://pod.ex/.acr";
    let mut nq = String::from(
        "<https://pod.ex/notes/n2#it> <https://ex.dev/ns#title> \"private\" <https://pod.ex/notes/n2> .\n",
    );
    for (s_, p, o) in [
        (g, "resource", "https://pod.ex/"),
        (g, "memberAccessControl", "https://pod.ex/.acr#c"),
        ("https://pod.ex/.acr#c", "apply", "https://pod.ex/.acr#p"),
        ("https://pod.ex/.acr#p", "deny", "READ"),
        ("https://pod.ex/.acr#p", "anyOf", "https://pod.ex/.acr#m"),
        ("https://pod.ex/.acr#m", "agent", BOB),
    ] {
        let o = if o == "READ" { format!("{ACL}Read") } else { o.to_owned() };
        nq += &format!("<{s_}> <{ACP}{p}> <{o}> <{g}> .\n");
    }
    let mut acp = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("loads"));
    acp.materialize_acp().expect("acp");
    acp.attach_odrl_policy(policy("permission", "read", "", None, Some(N2)).expect("policy"))
        .expect("attach");
    let carol = session(Some("https://carol.ex/card#me"), None);
    assert!(reads(&acp, &carol, N2), "the permission grants carol");
    assert!(!reads(&acp, &session(Some(BOB), None), N2), "bob's deny wins");
    assert!(!acp.decide(&session(Some(BOB), None), N2, Mode::Read).allow);

    let bare = "<https://pod.ex/notes/n2#it> <https://ex.dev/ns#title> \"x\" <https://pod.ex/notes/n2> .\n";
    let mut open = PodStore::new(Graph::load_dataset(bare, "nquads").expect("loads"));
    open.materialize_wac().expect("wac");
    open.attach_odrl_policy(policy("permission", "read", "", None, Some(N2)).expect("policy"))
        .expect("attach");
    assert!(!reads(&open, &carol, N2), "no ACL governs n2");
    assert!(!open.decide(&carol, N2, Mode::Read).allow);
}

/// A prohibition never touches `Control`, access-control documents or reserved graphs,
/// so even a targetless one cannot lock alice out of her own rules.
#[test]
fn control_and_rules_are_untouched() {
    let mut s = store();
    s.attach_odrl_policy(policy("prohibition", "use", "", Some(ALICE), None).expect("policy"))
        .expect("attach");
    let alice = session(Some(ALICE), None);
    assert!(!reads(&s, &alice, N2), "the asset itself is denied");
    assert!(!s.decide(&alice, N2, Mode::Write).allow);
    assert!(s.decide(&alice, N2, Mode::Control).allow, "Control stays");
    for acl in ["https://pod.ex/.acl", "https://pod.ex/notes/n1.acl"] {
        let read = s.decide(&alice, acl, Mode::Read).allow;
        let write = s.decide(&alice, acl, Mode::Write).allow;
        let before = bare_store();
        assert_eq!(read, before.decide(&alice, acl, Mode::Read).allow, "{acl}");
        assert_eq!(
            write,
            before.decide(&alice, acl, Mode::Write).allow,
            "{acl}"
        );
    }
    assert!(s.decide(&alice, "https://pod.ex/.acl", Mode::Write).allow);
    let acl_write = "INSERT DATA { GRAPH <https://pod.ex/.acl> { <urn:x> <urn:y> 1 } }";
    s.update_as(&alice, acl_write)
        .expect("alice still edits her rules");
}

/// Every mutation path and the create decision consult the prohibitions.
#[test]
fn mutations_and_creates_are_gated() {
    const CLEAR: &str = "CLEAR GRAPH <https://pod.ex/notes/n2>";
    let ins = "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 1 } }";
    let alice = session(Some(ALICE), None);
    let budget = QueryBudget::unlimited();
    let mut open = store();
    open.update_as(&alice, ins).expect("static grant writes");
    open.update_as(&alice, CLEAR).expect("static grant clears");
    assert!(open.decide_create(&alice, NOTES, "n3", Mode::Append).allow);

    for action in ["modify", "append", "use"] {
        let mut s = store();
        s.attach_odrl_policy(
            policy("prohibition", action, "", Some(ALICE), Some(N2)).expect("policy"),
        )
        .expect("attach");
        assert!(s.update_as(&alice, ins).is_err(), "update_as {action}");
        assert!(
            s.update_as_acp(&alice, ins).is_err(),
            "update_as_acp {action}"
        );
        assert!(
            s.update_as_with_budget(&alice, ins, &budget).is_err(),
            "budget {action}"
        );
        assert!(
            s.update_as_acp_with_budget(&alice, ins, &budget).is_err(),
            "acp budget {action}"
        );
        assert!(s.update_as(&alice, CLEAR).is_err(), "clear {action}");
        let d = s.decide_batch(
            &alice,
            &[(N2, Mode::Write), (N2, Mode::Append), (N2, Mode::Read)],
        );
        assert!(!d[0].allow && !d[1].allow, "decide_batch {action}");
        assert_eq!(
            d[2].allow,
            action != "use",
            "read is a different family for {action}"
        );
    }

    for target in [NOTES, "https://pod.ex/notes/n3"] {
        let mut s = store();
        s.attach_odrl_policy(
            policy("prohibition", "append", "", Some(ALICE), Some(target)).expect("policy"),
        )
        .expect("attach");
        assert!(
            !s.decide_create(&alice, NOTES, "n3", Mode::Append).allow,
            "{target}"
        );
        assert!(s.decide_create(&alice, NOTES, "n4", Mode::Append).allow == (target != NOTES));
        // The modes a create decision reports obey the same prohibitions as `allow`.
        for child in ["n3", "n4"] {
            let d = s.decide_create(&alice, NOTES, child, Mode::Append);
            let barred = target == NOTES || child == "n3";
            for m in [Mode::Append, Mode::Write] {
                assert_eq!(d.granted_modes.contains(&m), !barred, "{target} {child} {m:?}");
            }
            assert!(d.granted_modes.contains(&Mode::Read), "{target} {child}");
        }
    }
}

/// `WAC-Allow`'s public field is computed at the request clock, like its user field.
#[test]
fn wac_allow_keeps_the_request_clock() {
    let window = "odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ; \
                  odrl:rightOperand \"2030-01-01T00:00:00Z\"^^xsd:dateTime";
    let mut s = store();
    s.attach_odrl_policy(policy("prohibition", "read", window, None, Some(N1)).expect("policy"))
        .expect("attach");
    let n1 = NamedNode::new(N1).expect("iri");
    assert_eq!(
        s.wac_allow(&session(None, Some("2035-06-01T00:00:00Z")), &n1),
        r#"user="read",public="read""#
    );
    assert_eq!(
        s.wac_allow(&session(None, Some("2025-06-01T00:00:00Z")), &n1),
        r#"user="",public="""#
    );
    assert_eq!(
        s.wac_allow(&session(None, None), &n1),
        r#"user="",public="""#
    );
    #[cfg(feature = "pattern-scope")]
    {
        let scoped = s.scoped_dataset(&session(None, None), Mode::Read, &Default::default());
        assert!(
            scoped.view().named.is_empty(),
            "scoped_dataset reads the same set"
        );
    }
}

/// Every public `sparq-solid` function that takes a [`Session`] is either gated (and
/// exercised above) or named here with the reason it is not an access decision. A new
/// entry point fails this test until it is classified.
#[test]
fn every_session_entry_point_is_classified() {
    const GATED: &[&str] = &[
        "accessible",
        "accessible_set",
        "wac_allow",
        "decide",
        "decide_batch",
        "decide_create",
        "decide_removal",
        "view_for",
        "query_as",
        "query_json_as",
        "ask_as",
        "query_as_rewrite",
        "update_as",
        "update_as_acp",
        "update_as_with_budget",
        "update_as_acp_with_budget",
        "scoped_dataset",
    ];
    // `Session::at` builds a session; `AuthIndex::accessible` is the raw static index
    // that `PodStore::accessible` narrows.
    const NOT_DECISIONS: &[&str] = &["at", "accessible"];
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(src).expect("src") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).expect("read");
            found.extend(session_fns(&text));
        }
    }
    for name in &found {
        assert!(
            GATED.contains(&name.as_str()) || NOT_DECISIONS.contains(&name.as_str()),
            "pub fn {name} takes a Session but is not classified"
        );
    }
    for name in GATED {
        assert!(
            found.iter().any(|f| f == name),
            "{name} is no longer an entry point"
        );
    }
}

/// Names of `pub fn`s whose parameter list mentions the `Session` type.
fn session_fns(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (at, _) in text.match_indices("pub fn ") {
        let rest = &text[at + 7..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let Some(open) = rest.find('(') else { continue };
        let mut depth = 0;
        let mut close = open;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let params = &rest[open..close];
        let mentions = params.match_indices("Session").any(|(i, _)| {
            let before = params[..i].chars().next_back();
            let after = params[i + 7..].chars().next();
            !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
        });
        if mentions {
            out.push(name);
        }
    }
    out
}

/// A request carries no party-membership evidence, so a prohibition on a party
/// collection applies to every authenticated agent (a member could be any of them), and
/// one excluding the collection still applies to them (any of them could be outside it).
#[test]
fn party_collection_prohibitions_fail_closed() {
    let alice = session(Some(ALICE), None);
    let on_team = policy("prohibition", "read", "", Some(TEAM), Some(N2)).expect("policy");
    let mut s = store();
    s.attach_odrl_policy(on_team).expect("attach");
    assert!(!reads(&s, &alice, N2), "a team prohibition must reach a possible member");
    assert!(reads(&s, &session(None, None), N1), "anonymous is no member");

    let off_team = policy(
        "prohibition",
        "read",
        "odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ; odrl:rightOperand <https://pod.ex/team>",
        None,
        Some(N2),
    )
    .expect("policy");
    let mut s = store();
    s.attach_odrl_policy(off_team).expect("attach");
    assert!(!reads(&s, &alice, N2), "an exclusion must reach a possible non-member");
}

/// A read prohibition also confines what an update's WHERE can read: a conditional write
/// cannot copy prohibited data into a graph the session may read back.
#[test]
fn read_prohibitions_reach_update_conditions() {
    const OUT: &str = "https://pod.ex/notes/out";
    let alice = session(Some(ALICE), None);
    let mut s = bare_store_with(&format!("<urn:o> <urn:p> \"out\" <{OUT}> .\n"));
    s.attach_odrl_policy(policy("prohibition", "read", "", Some(ALICE), Some(N2)).expect("policy"))
        .expect("attach");
    assert!(
        s.update_as(
            &alice,
            &format!("INSERT {{ GRAPH <{OUT}> {{ ?s ?p ?o }} }} WHERE {{ GRAPH <{N2}> {{ ?s ?p ?o }} }}"),
        )
        .is_err(),
        "a constant prohibited source is refused"
    );
    s.update_as(
        &alice,
        &format!("INSERT {{ GRAPH <{OUT}> {{ ?s ?p ?o }} }} WHERE {{ GRAPH ?g {{ ?s ?p ?o }} }}"),
    )
    .expect("a variable source ranges over readable graphs");
    let copied = s
        .query_json_as(&alice, Mode::Read, &format!("SELECT ?o WHERE {{ GRAPH <{OUT}> {{ ?s ?p ?o }} }}"))
        .expect("query");
    assert!(copied.contains("hello"), "readable data is copied: {copied}");
    assert!(!copied.contains("private"), "prohibited data leaked: {copied}");
}
