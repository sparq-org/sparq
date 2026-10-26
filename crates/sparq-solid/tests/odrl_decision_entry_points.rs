//! Every grant goes through one decision: sparq-policy's `decide`. This runs each
//! undecidable constraint shape, on both rule kinds and under every wrapper, through each
//! entry point that can put an allow into the auth view: the evaluator, the one-shot,
//! policy and conditional ACP materialisers, the N3 materialiser, a ledger replay of a
//! grant that was live before the policy changed, and the request-time layer of an
//! attached policy, for the deciding session and for others. None may grant.
//!
//! Gated by the `odrl-bridge` feature (the whole test file no-ops without it).
#![cfg(feature = "odrl-bridge")]

use sparq_core::Graph;
use sparq_policy::{evaluate, parse_policy_str, Request, ValidatedPolicy};
use sparq_solid::odrl_bridge::materialize_odrl_n3;
use sparq_solid::{BridgeKind, Mode, PodStore, Session};

const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const CAROL: &str = "https://carol.ex/card#me";
const N1: &str = "https://pod.ex/notes/n1";
const NOW: &str = "2026-06-01T00:00:00Z";

/// A constraint the request below definitely satisfies, and one it definitely fails.
const TRUE_C: &str =
    "[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/a> ]";
const FALSE_C: &str =
    "[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/z> ]";

/// Shapes the evaluator cannot decide for [`request`].
const SHAPES: [(&str, &str); 9] = [
    (
        "two dateTimes under lteq",
        r#"[ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
             odrl:rightOperand "2026-12-31T00:00:00Z"^^xsd:dateTime , "2027-12-31T00:00:00Z"^^xsd:dateTime ]"#,
    ),
    (
        "two purposes under eq",
        "[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/a> , <urn:p/b> ]",
    ),
    ("missing rightOperand", "[ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ]"),
    (
        "unknown operator",
        "[ odrl:leftOperand odrl:purpose ; odrl:operator <urn:op/unknown> ; odrl:rightOperand <urn:p/a> ]",
    ),
    (
        "typed set membership",
        "[ odrl:leftOperand odrl:count ; odrl:operator odrl:isAnyOf ; odrl:rightOperand ( 1 2 ) ]",
    ),
    (
        "unsupported unit",
        "[ odrl:leftOperand odrl:count ; odrl:operator odrl:lteq ; odrl:rightOperand 5 ; odrl:unit <urn:unit/kb> ]",
    ),
    (
        "incomparable operand types",
        "[ odrl:leftOperand odrl:count ; odrl:operator odrl:eq ; odrl:rightOperand <urn:p/a> ]",
    ),
    (
        "unparseable dateTime bound",
        r#"[ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ; odrl:rightOperand "soon"^^xsd:dateTime ]"#,
    ),
    (
        "no evidence for the dimension",
        "[ odrl:leftOperand odrl:spatial ; odrl:operator odrl:eq ; odrl:rightOperand <urn:region/eu> ]",
    ),
];

const PREFIXES: &str = "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n\
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n";

fn request() -> Request {
    Request::new("http://www.w3.org/ns/odrl/2/read")
        .on(N1)
        .by(ALICE)
        .at(NOW)
        .for_purpose(sparq_policy::Value::Iri("urn:p/a".into()))
        .with(
            "http://www.w3.org/ns/odrl/2/count",
            sparq_policy::Value::Num(2.0),
        )
}

fn pod() -> Graph {
    Graph::load_dataset(
        "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/notes/n1> .",
        "nquads",
    )
    .expect("pod loads")
}

fn alice_reads(store: &PodStore) -> bool {
    reads_as(store, ALICE)
}

fn reads_as(store: &PodStore, agent: &str) -> bool {
    let s = Session {
        agent: Some(agent),
        client: None,
        issuer: None,
        now: Some(NOW),
    };
    store
        .accessible(&s, Mode::Read)
        .iter()
        .any(|g| g.as_str() == N1)
        || store.decide(&s, N1, Mode::Read).allow
}

/// [`pod`] with an ACL on n1 that grants only carol, so decisions on n1 are
/// authoritative and an attached policy can grant there.
fn governed_pod() -> PodStore {
    let acl = "https://pod.ex/notes/n1.acl";
    let a = "https://pod.ex/notes/n1.acl#a";
    let nq = format!(
        "<{a}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <{acl}> .\n\
         <{a}> <http://www.w3.org/ns/auth/acl#accessTo> <{N1}> <{acl}> .\n\
         <{a}> <http://www.w3.org/ns/auth/acl#agent> <{CAROL}> <{acl}> .\n\
         <{a}> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <{acl}> .\n"
    );
    let pod = format!("<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <{N1}> .\n{nq}");
    let mut store = PodStore::new(Graph::load_dataset(&pod, "nquads").expect("pod loads"));
    store.materialize_wac().expect("wac");
    store
}

/// The request-time layer of an attached policy, for alice and for bob.
fn attached_grants(policy: &ValidatedPolicy) -> [bool; 2] {
    let mut store = governed_pod();
    store.attach_odrl_policy(policy.clone()).expect("attach");
    [reads_as(&store, ALICE), reads_as(&store, BOB)]
}

/// The constraint object for `shape` under each wrapper.
fn wrapped(shape: &str) -> [(&'static str, String); 4] {
    let lc = |op: &str, other: &str| {
        format!("[ a odrl:LogicalConstraint ; odrl:{op} ( {shape} {other} ) ]")
    };
    [
        ("atomic", shape.to_owned()),
        ("and", lc("and", TRUE_C)),
        ("or", lc("or", FALSE_C)),
        ("xone", lc("xone", TRUE_C)),
    ]
}

fn permission(c: &str) -> String {
    format!(
        "{PREFIXES}<urn:pol/p> a odrl:Set ; odrl:permission [ odrl:action odrl:read ; \
         odrl:target <{N1}> ; odrl:assignee <{ALICE}> ; odrl:constraint {c} ] ."
    )
}

fn prohibition(c: &str) -> String {
    format!(
        "{PREFIXES}<urn:pol/p> a odrl:Set ; \
         odrl:permission [ odrl:action odrl:read ; odrl:target <{N1}> ; odrl:assignee <{ALICE}> ] ; \
         odrl:prohibition [ odrl:action odrl:read ; odrl:target <{N1}> ; odrl:assignee <{ALICE}> ; \
         odrl:constraint {c} ] ."
    )
}

/// An unconstrained grant for alice: what is live before the policy changes.
fn granting_ttl() -> String {
    format!(
        "{PREFIXES}<urn:pol/p> a odrl:Set ; odrl:permission [ odrl:action odrl:read ; \
         odrl:target <{N1}> ; odrl:assignee <{ALICE}> ] ."
    )
}

fn granting() -> ValidatedPolicy {
    parse_policy_str(&granting_ttl(), "turtle").expect("granting policy parses")
}

/// Assert no entry point grants alice's read under `ttl`.
fn no_entry_point_grants(ttl: &str, req: &Request, what: &str) {
    // The N3 path parses the raw document itself.
    let mut g = pod();
    if let Ok(out) = materialize_odrl_n3(&mut g, ttl, req) {
        assert!(!out.granted, "N3 granted: {what}");
    }
    let Ok(policy) = parse_policy_str(ttl, "turtle") else {
        return; // refused at the boundary: nothing downstream can see it
    };
    assert!(!evaluate(&policy, req).allow, "evaluate granted: {what}");
    assert_eq!(attached_grants(&policy), [false, false], "request-time layer granted: {what}");

    let mut store = PodStore::new(pod());
    store.materialize_odrl_permission(&policy, req).unwrap();
    assert!(!alice_reads(&store), "one-shot granted: {what}");
    let mut store = PodStore::new(pod());
    store.materialize_odrl_policy(&policy, req).unwrap();
    assert!(!alice_reads(&store), "policy materialiser granted: {what}");
    let mut store = PodStore::new(pod());
    store.materialize_odrl_permission_conditional(&policy, req).unwrap();
    assert!(
        !alice_reads(&store),
        "conditional materialiser granted: {what}"
    );

    // Replay: a grant that was live under the old policy does not survive a refresh
    // against this one.
    for (kind, label) in [
        (BridgeKind::Permission, "one-shot"),
        (BridgeKind::Policy, "policy"),
        (BridgeKind::PermissionConditional, "conditional"),
    ] {
        let mut store = PodStore::new(pod());
        match kind {
            BridgeKind::Permission => store.materialize_odrl_permission(&granting(), req).unwrap(),
            BridgeKind::Policy => store.materialize_odrl_policy(&granting(), req).unwrap(),
            _ => store.materialize_odrl_permission_conditional(&granting(), req).unwrap(),
        };
        assert!(
            alice_reads(&store),
            "{label} grant is live before the change: {what}"
        );
        store.refresh_odrl_grant(&policy, req, kind).unwrap();
        assert!(
            !alice_reads(&store),
            "{label} replay kept the grant: {what}"
        );
    }
}

#[test]
fn no_entry_point_grants_an_undecidable_shape() {
    for (name, shape) in SHAPES {
        for (wrapper, c) in wrapped(shape) {
            no_entry_point_grants(
                &permission(&c),
                &request(),
                &format!("permission, {wrapper}, {name}"),
            );
            no_entry_point_grants(
                &prohibition(&c),
                &request(),
                &format!("prohibition, {wrapper}, {name}"),
            );
        }
    }
}

/// A constrained duty is never discharged, whichever path reads it.
#[test]
fn no_entry_point_discharges_a_constrained_duty_by_its_action() {
    let ttl = format!(
        "{PREFIXES}<urn:pol/p> a odrl:Set ; odrl:permission [ odrl:action odrl:read ; \
         odrl:target <{N1}> ; odrl:assignee <{ALICE}> ; odrl:duty [ odrl:action odrl:compensate ; \
         odrl:constraint [ odrl:leftOperand odrl:payAmount ; odrl:operator odrl:eq ; \
         odrl:rightOperand 5 ] ] ] ."
    );
    let req = request().discharge("http://www.w3.org/ns/odrl/2/compensate");
    no_entry_point_grants(&ttl, &req, "constrained duty");
}

/// A dateTime prohibition with no time evidence is Unknown, so it blocks on every path,
/// including N3, which derives no satisfaction for it.
#[test]
fn no_entry_point_grants_past_a_prohibition_missing_its_evidence() {
    let ttl = prohibition(
        r#"[ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
             odrl:rightOperand "2026-12-31T00:00:00Z"^^xsd:dateTime ]"#,
    );
    let req = Request::new("http://www.w3.org/ns/odrl/2/read")
        .on(N1)
        .by(ALICE);
    no_entry_point_grants(&ttl, &req, "dateTime prohibition, no clock");
}

/// The control: a decidable policy still grants. `evaluate` grants past a prohibition
/// whose window definitely closed; every stored-grant path, N3 included, grants the one
/// lasting shape (an unconstrained grant to alice, in a policy with no prohibitions).
#[test]
fn a_decidable_grant_still_reaches_every_entry_point() {
    let ttl = prohibition(
        r#"[ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
             odrl:rightOperand "2026-01-01T00:00:00Z"^^xsd:dateTime ]"#,
    );
    let policy = parse_policy_str(&ttl, "turtle").unwrap();
    assert!(evaluate(&policy, &request()).allow);
    let mut g = pod();
    assert!(
        materialize_odrl_n3(&mut g, &granting_ttl(), &request())
            .unwrap()
            .granted
    );
    let mut store = PodStore::new(pod());
    store.materialize_odrl_permission(&granting(), &request()).unwrap();
    assert!(alice_reads(&store));
    let mut store = PodStore::new(pod());
    store.materialize_odrl_policy(&granting(), &request()).unwrap();
    assert!(alice_reads(&store));
    let mut store = PodStore::new(pod());
    store.materialize_odrl_permission_conditional(&granting(), &request()).unwrap();
    assert!(alice_reads(&store));
    assert_eq!(attached_grants(&granting()), [true, false], "only alice is the assignee");
}
