//! Every definite verdict of the static comparison checked against `decide`.
//!
//! Policy pairs are generated over a small vocabulary (actions inside and outside
//! `use`, pinned and open targets and assignees, widened and flat constraints, duties,
//! prohibitions, a declared party collection), and requests are sampled with and
//! without evidence and membership. Whenever `contains` or `detect_conflicts` gives a
//! definite verdict, the sampled requests must agree with it:
//!
//! - `Contains`: no request `inner` grants is refused by `outer`.
//! - `NotContained`: some request `inner` grants is refused by `outer`.
//! - `Overlap::Certain`: no request the permission grants alone survives the
//!   prohibition.

use sparq_policy::{
    contains, decide, detect_conflicts, parse_policy_str, Containment, Overlap, Policy, Request,
    ValidatedPolicy, Value,
};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
const ACTIONS: [&str; 5] = ["use", "read", "print", "sell", "modify"];
const TARGETS: [&str; 4] = ["urn:asset/x", "urn:asset/y", "urn:sparq:compare:any-asset", "urn:asset/z"];
const PARTIES: [&str; 4] =
    ["urn:alice", "urn:bob", "urn:sparq:compare:any-party", "urn:carol"];

/// A small deterministic generator (no extra dev-dependency).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() as usize) % xs.len()]
    }
    fn one_in(&mut self, n: u64) -> bool {
        self.next().is_multiple_of(n)
    }
}

fn rule(g: &mut Lcg) -> String {
    let action = g.pick(&["use", "read", "print", "sell"]);
    let mut body = format!("odrl:action odrl:{action}");
    if let Some(t) = g.pick(&[None, Some("urn:asset/x"), Some("urn:asset/y")]) {
        body += &format!(" ; odrl:target <{t}>");
    }
    if let Some(a) = g.pick(&[None, Some("urn:alice"), Some("urn:bob")]) {
        body += &format!(" ; odrl:assignee <{a}>");
    }
    let constraint = g.pick(&[
        "",
        "",
        "dateTime lt \"2026-06-01T00:00:00Z\"^^xsd:dateTime",
        "dateTime gt \"2026-01-01T00:00:00Z\"^^xsd:dateTime",
        "purpose eq <urn:p/a>",
        "purpose isAnyOf \"urn:p/a|urn:p/b\"",
    ]);
    if !constraint.is_empty() {
        let mut parts = constraint.splitn(3, ' ');
        let (l, o, r) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
        body += &format!(
            " ; odrl:constraint [ odrl:leftOperand odrl:{l} ; odrl:operator odrl:{o} ; \
             odrl:rightOperand {r} ]"
        );
    }
    body
}

fn policy(g: &mut Lcg, id: usize) -> ValidatedPolicy {
    let mut rules = Vec::new();
    for _ in 0..1 + g.next() % 2 {
        let mut perm = rule(g);
        if g.one_in(8) {
            perm += " ; odrl:duty [ odrl:action odrl:attribute ]";
        } else if g.one_in(8) {
            perm += " ; odrl:duty [ odrl:action odrl:compensate ; odrl:constraint [ \
                     odrl:leftOperand odrl:payAmount ; odrl:operator odrl:eq ; \
                     odrl:rightOperand 5 ] ]";
        }
        rules.push(format!("odrl:permission [ {perm} ]"));
    }
    if g.one_in(3) {
        rules.push(format!("odrl:prohibition [ {} ]", rule(g)));
    }
    let mut ttl = format!(
        "@prefix odrl: <{ODRL}> .\n@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
         <urn:pol/{id}> a odrl:Set ;\n{} .\n",
        rules.join(" ;\n")
    );
    if g.one_in(10) {
        ttl += "<urn:bob> a odrl:PartyCollection .\n";
    }
    parse_policy_str(&ttl, "turtle").unwrap_or_else(|e| panic!("{e}\n{ttl}"))
}

/// Every action/target/party combination with no evidence (the space a witness comes
/// from), plus requests carrying a clock, a purpose, subsumption and membership.
fn requests(g: &mut Lcg) -> Vec<Request> {
    let mut out = Vec::new();
    for a in ACTIONS {
        for t in TARGETS {
            for p in PARTIES {
                out.push(Request::new(format!("{ODRL}{a}")).on(t).by(p));
            }
        }
    }
    for _ in 0..120 {
        let mut r = Request::new(format!("{ODRL}{}", g.pick(&ACTIONS)))
            .on(*g.pick(&TARGETS))
            .by(*g.pick(&PARTIES));
        if let Some(at) = g.pick(&[None, Some("2026-03-01T00:00:00Z"), Some("2026-09-01T00:00:00Z")]) {
            r = r.at(*at);
        }
        if let Some(p) = g.pick(&[None, Some("urn:p/a"), Some("urn:p/c")]) {
            r = r.for_purpose(Value::Iri((*p).into()));
        }
        if g.one_in(2) {
            r = r.with_purpose_subsumption("urn:p/c", "urn:p/a");
        }
        if g.one_in(2) {
            r = r.with_party_membership(*g.pick(&PARTIES), *g.pick(&["urn:alice", "urn:bob"]));
        }
        if g.one_in(2) {
            r = r.with_asset_membership(*g.pick(&TARGETS), *g.pick(&["urn:asset/x", "urn:asset/y"]));
        }
        if g.one_in(2) {
            r = r.discharge(format!("{ODRL}attribute"));
        }
        out.push(r);
    }
    out
}

fn only(perms: Vec<sparq_policy::Rule>, prohs: Vec<sparq_policy::Rule>) -> ValidatedPolicy {
    Policy { permissions: perms, prohibitions: prohs, ..Policy::default() }
        .validate()
        .expect("sub-policy validates")
}

#[test]
fn definite_verdicts_agree_with_decide() {
    let mut g = Lcg(0x5eed_6734);
    let (mut contains_n, mut not_contained_n, mut certain_n) = (0, 0, 0);
    for i in 0..1000 {
        let outer = policy(&mut g, 2 * i);
        let inner = policy(&mut g, 2 * i + 1);
        let reqs = requests(&mut g);
        match contains(&outer, &inner) {
            Containment::Contains => {
                contains_n += 1;
                for r in &reqs {
                    assert!(
                        !decide(&inner, r).allow || decide(&outer, r).allow,
                        "Contains refuted by {r:?}\nouter {outer:?}\ninner {inner:?}"
                    );
                }
            }
            Containment::NotContained => {
                not_contained_n += 1;
                assert!(
                    reqs.iter().any(|r| decide(&inner, r).allow && !decide(&outer, r).allow),
                    "NotContained without a witness\nouter {outer:?}\ninner {inner:?}"
                );
            }
            Containment::Unknown => {}
        }
        for c in detect_conflicts(&inner) {
            if c.overlap != Overlap::Certain {
                continue;
            }
            certain_n += 1;
            let perm = inner.permissions.iter().find(|r| r.id == c.permission_id).unwrap();
            let proh = inner.prohibitions.iter().find(|r| r.id == c.prohibition_id).unwrap();
            let alone = only(vec![perm.clone()], vec![]);
            let both = only(vec![perm.clone()], vec![proh.clone()]);
            for r in &reqs {
                assert!(
                    !decide(&alone, r).allow || !decide(&both, r).allow,
                    "Certain conflict survived by {r:?}\nperm {perm:?}\nproh {proh:?}"
                );
            }
        }
    }
    // The generator must exercise every definite verdict, not only Unknown.
    assert!(contains_n >= 10, "only {contains_n} Contains verdicts");
    assert!(not_contained_n >= 10, "only {not_contained_n} NotContained verdicts");
    assert!(certain_n >= 10, "only {certain_n} Certain conflicts");
}
