//! [OPUS-4.8] sq-zgbso.1 — CI enforcement of the ODRL-as-N3 spike's load-bearing
//! invariant: for the representative ODRL permission-evaluation case, the `auth:*`
//! decision SET derived by the N3 rules (`rules/odrl-spike.n3`) MUST EQUAL the SET
//! materialised by the real Rust path (`odrl_bridge::materialize_policy` over
//! `sparq_policy::evaluate`). The `examples/odrl_n3_spike.rs` harness ALSO prints the
//! (non-canonical) timing / parse-fraction / build-size measurements; this test locks
//! the differential so a future rules or evaluator edit cannot silently break parity.
//!
//! Gated behind the default-OFF `odrl-bridge` feature (it drives the real ODRL
//! evaluator), so the default build compiles none of it.
#![cfg(feature = "odrl-bridge")]

use oxrdf::Term;
use sparq_core::dict::Dict;
use sparq_core::Graph;
use sparq_policy::{parse_policy_str, Request};
use sparq_reason::reason_n3;
use sparq_solid::odrl_bridge::materialize_policy;
use sparq_solid::AUTH_NS;

const RULES: &str = include_str!("../rules/odrl-spike.n3");
const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

type AuthSet = Vec<(String, String, String)>;

const POLICY_A: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/a> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
                      odrl:rightOperand "2026-12-31T00:00:00Z"^^xsd:dateTime ] ] ."#;

const POLICY_B: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/b> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ."#;

const POLICY_C: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/c> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ."#;

fn rust_set(policy_ttl: &str, action_local: &str, target: &str, party: &str, at: Option<&str>) -> AuthSet {
    let policy = parse_policy_str(policy_ttl, "turtle").expect("policy parses");
    let mut req = Request::new(format!("{}{}", ODRL, action_local)).on(target).by(party);
    if let Some(t) = at {
        req = req.at(t);
    }
    let mut graph = Graph::new();
    let outcome = materialize_policy(&mut graph, &policy, &req);
    let mut set: AuthSet = Vec::new();
    if let Some(t) = outcome.grant_triple {
        set.push(t);
    }
    if let Some(t) = outcome.deny_triple {
        set.push(t);
    }
    set.sort();
    set
}

fn n3_set(policy_ttl: &str, action_local: &str, target: &str, party: &str, at: Option<&str>) -> AuthSet {
    let at_clause = match at {
        Some(t) => format!(" ; spike:atTime \"{}\"^^xsd:dateTime", t),
        None => String::new(),
    };
    let request = format!(
        "<urn:req> a odrl:Request ; odrl:action odrl:{} ; odrl:target <{}> ; odrl:assignee <{}>{} .",
        action_local, target, party, at_clause
    );
    let src = format!(
        "@prefix odrl: <{}> .\n@prefix spike: <https://sparq.dev/ns/odrl-spike#> .\n\
         @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n{}\n{}\n{}",
        ODRL, policy_ttl, request, RULES
    );
    let mut dict = Dict::new();
    let closure = reason_n3(&mut dict, &src).expect("N3 reasons");
    let mut set: AuthSet = Vec::new();
    for t in &closure {
        let p = dict.term(t[1]);
        let Term::NamedNode(pred) = &p else { continue };
        if !pred.as_str().starts_with(AUTH_NS) {
            continue;
        }
        let (Term::NamedNode(subj), Term::NamedNode(obj)) = (dict.term(t[0]), dict.term(t[2])) else {
            continue;
        };
        set.push((subj.as_str().to_owned(), pred.as_str().to_owned(), obj.as_str().to_owned()));
    }
    set.sort();
    set
}

fn expect(pairs: &[(&str, &str, &str)]) -> AuthSet {
    let mut s: AuthSet = pairs
        .iter()
        .map(|(a, m, t)| ((*a).to_owned(), format!("{}{}", AUTH_NS, m), (*t).to_owned()))
        .collect();
    s.sort();
    s
}

/// Assert Rust == N3 == the independently-stated expected set (so a both-wrong
/// agreement cannot pass), for one scenario.
fn assert_case(
    label: &str,
    policy: &str,
    action: &str,
    party: &str,
    at: Option<&str>,
    want: &[(&str, &str, &str)],
) {
    let rust = rust_set(policy, action, "urn:t/1", party, at);
    let n3 = n3_set(policy, action, "urn:t/1", party, at);
    let expected = expect(want);
    assert_eq!(rust, expected, "{}: Rust path != expected", label);
    assert_eq!(n3, expected, "{}: N3 path != expected", label);
    assert_eq!(rust, n3, "{}: Rust and N3 diverge", label);
}

#[test]
fn a1_allow_within_datetime_window() {
    // The spike's rules derive the grant inside the window, but the bridge stores
    // nothing: the window closes and a stored triple is never re-checked against the
    // clock.
    let at = Some("2026-07-05T00:00:00Z");
    let want = expect(&[("urn:alice", "read", "urn:t/1")]);
    assert_eq!(n3_set(POLICY_A, "read", "urn:t/1", "urn:alice", at), want, "A1: N3 rules");
    assert!(rust_set(POLICY_A, "read", "urn:t/1", "urn:alice", at).is_empty(), "A1: bridge");
}

#[test]
fn a2_deny_after_window_fail_closed() {
    assert_case("A2", POLICY_A, "read", "urn:alice", Some("2027-01-01T00:00:00Z"), &[]);
}

#[test]
fn a3_deny_no_time_unprovable_fail_closed() {
    assert_case("A3", POLICY_A, "read", "urn:alice", None, &[]);
}

#[test]
fn a4_deny_wrong_assignee_fail_closed() {
    assert_case("A4", POLICY_A, "read", "urn:bob", Some("2026-07-05T00:00:00Z"), &[]);
}

#[test]
fn a5_deny_action_mismatch_fail_closed() {
    assert_case("A5", POLICY_A, "write", "urn:alice", Some("2026-07-05T00:00:00Z"), &[]);
}

#[test]
fn b1_allow_unconstrained_permission() {
    assert_case("B1", POLICY_B, "read", "urn:alice", None, &[("urn:alice", "read", "urn:t/1")]);
}

#[test]
fn c1_deny_overrides_prohibition_wins() {
    assert_case("C1", POLICY_C, "read", "urn:alice", None, &[("urn:alice", "denyRead", "urn:t/1")]);
}
