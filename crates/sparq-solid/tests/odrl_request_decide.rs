//! Request-time ODRL prohibitions (issue #6743) agree with `matched_prohibition` for
//! every session, and only ever narrow the static decision.
//!
//! Permissions and prohibitions are generated over every constraint left operand the
//! evaluator knows (and one it does not), compound constraints, assignees (a named
//! agent, one no request names, a party collection, wildcard principals) and targets
//! (an asset, an asset collection holding both assets, no target, an unrelated asset).
//! Each policy is attached to a store where n1 is publicly readable and n2 is readable
//! by alice alone. For every session (named parties, an unseen party, anonymous) and
//! clock:
//!
//! - a graph is readable exactly when the static view grants it and no prohibition
//!   applies to any read-family action (True or Unknown), through `accessible`,
//!   `query_as` and the point `decide`; an attached permission never adds access;
//! - the request-time layer never denies a session on n1 that the materialized
//!   conditional deny for the same policy lets through.
//!
//! The remaining tests pin the mutation paths, the graphs the layer never touches, the
//! `WAC-Allow` clock, and an enumeration of every entry point that takes a session.
#![cfg(feature = "odrl-bridge")]

use oxrdf::NamedNode;
use sparq_core::Graph;
use sparq_engine::QueryBudget;
use sparq_policy::{matched_prohibition, parse_policy_str, Request, ValidatedPolicy};
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

/// The static read verdict, narrowed by every read-family prohibition that applies.
fn expected(pol: &ValidatedPolicy, graph: &str, agent: Option<&str>, now: Option<&str>) -> bool {
    let prohibited = READ_ACTIONS
        .iter()
        .any(|a| applies(pol, request(a, graph, agent, now), agent));
    let static_read = graph == N1 || agent == Some(ALICE);
    static_read && !prohibited
}

#[test]
fn request_time_decisions_match_decide_for_every_session() {
    let (mut cases, mut narrowed, mut denied) = (0, 0, 0);
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
                                    let static_read = graph == N1 || agent == Some(ALICE);
                                    denied += usize::from(static_read && !want);
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
    assert!(denied >= 1_000, "only {denied} static grants were narrowed");
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

/// An attached permission grants nothing: not on a graph the static view refuses, not
/// on an access-control document, not on a reserved graph, and not for a write.
#[test]
fn permissions_never_grant() {
    let mut s = store();
    let targets: [Option<&str>; 4] = [
        None,
        Some(N2),
        Some("https://pod.ex/notes/n1.acl"),
        Some("urn:sparq:auth"),
    ];
    for target in targets {
        s.attach_odrl_policy(policy("permission", "use", "", None, target).expect("policy"))
            .expect("attach");
    }
    let bob = session(Some("https://bob.ex/card#me"), None);
    let before = bare_store();
    for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
        assert_eq!(
            s.accessible(&bob, mode),
            before.accessible(&bob, mode),
            "{mode:?}"
        );
    }
    for graph in [
        N2,
        "https://pod.ex/notes/n1.acl",
        "https://pod.ex/.acl",
        "urn:sparq:auth",
    ] {
        for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
            assert!(!s.decide(&bob, graph, mode).allow, "{graph} {mode:?}");
        }
    }
    let ins = "INSERT DATA { GRAPH <https://pod.ex/notes/n2> { <urn:x> <urn:y> 1 } }";
    assert!(s.update_as(&bob, ins).is_err());
    assert!(!s.decide_create(&bob, NOTES, "n3", Mode::Append).allow);
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
