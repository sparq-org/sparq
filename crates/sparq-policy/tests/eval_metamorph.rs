//! Deterministic metamorphic tests for the evaluator's deny invariants.

use sparq_policy::{evaluate, parse_policy_str, Request};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
const ACTIONS: [&str; 3] = ["read", "write", "display"];
const TARGETS: [&str; 3] = ["urn:asset/a", "urn:asset/b", "urn:asset/c"];
const PARTIES: [&str; 3] = ["urn:party/alice", "urn:party/bob", "urn:party/carol"];

/// Tiny deterministic generator: enough variation for these algebraic tests
/// without adding a random-number dependency to the crate.
struct Generator(u64);

impl Generator {
    fn next(&mut self, upper: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 as usize) % upper
    }
}

fn rule(kind: &str, id: usize, action: &str, target: &str, party: &str) -> String {
    format!(
        "odrl:{kind} [ odrl:uid <urn:rule/{id}> ; odrl:action odrl:{action} ; \
         odrl:target <{target}> ; odrl:assignee <{party}> ]"
    )
}

fn policy(rules: &[String]) -> String {
    let body = rules.join(" ;\n  ");
    format!(
        "@prefix odrl: <{ODRL}> .\n<urn:policy/generated> a odrl:Set{}{} .\n",
        if body.is_empty() { "" } else { " ;\n  " },
        body
    )
}

fn request(action: &str, target: &str, party: &str) -> Request {
    Request::new(format!("{ODRL}{action}")).on(target).by(party)
}

#[test]
fn prohibition_overrides_permission() {
    let mut generator = Generator(0x5a17_d3c4_91e2_b687);

    for case in 0..64 {
        let chosen = (
            ACTIONS[generator.next(ACTIONS.len())],
            TARGETS[generator.next(TARGETS.len())],
            PARTIES[generator.next(PARTIES.len())],
        );
        let mut rules = vec![rule("permission", case * 16, chosen.0, chosen.1, chosen.2)];

        for offset in 1..=generator.next(7) {
            let kind = if generator.next(4) == 0 {
                "prohibition"
            } else {
                "permission"
            };
            let candidate = (
                ACTIONS[generator.next(ACTIONS.len())],
                TARGETS[generator.next(TARGETS.len())],
                PARTIES[generator.next(PARTIES.len())],
            );
            // Preserve the generated policy's ALLOW precondition.
            if kind == "prohibition" && candidate == chosen {
                continue;
            }
            rules.push(rule(
                kind,
                case * 16 + offset,
                candidate.0,
                candidate.1,
                candidate.2,
            ));
        }

        let req = request(chosen.0, chosen.1, chosen.2);
        let before = parse_policy_str(&policy(&rules), "turtle").unwrap();
        assert!(evaluate(&before, &req).allow, "generated case {case}");

        rules.push(rule(
            "prohibition",
            case * 16 + 15,
            chosen.0,
            chosen.1,
            chosen.2,
        ));
        let after = parse_policy_str(&policy(&rules), "turtle").unwrap();
        assert!(
            !evaluate(&after, &req).allow,
            "matching prohibition did not override generated case {case}"
        );
    }
}

#[test]
fn no_match_is_fail_closed_deny() {
    let mut generator = Generator(0xc012_5eed_7a11_f00d);

    for case in 0..64 {
        let mut rules = Vec::new();
        for offset in 0..generator.next(8) {
            let kind = if generator.next(2) == 0 {
                "permission"
            } else {
                "prohibition"
            };
            rules.push(rule(
                kind,
                case * 8 + offset,
                ACTIONS[generator.next(ACTIONS.len())],
                TARGETS[generator.next(TARGETS.len())],
                PARTIES[generator.next(PARTIES.len())],
            ));
        }

        let generated = parse_policy_str(&policy(&rules), "turtle").unwrap();
        let unmatched = request("archive", "urn:asset/absent", "urn:party/nobody");
        assert!(
            !evaluate(&generated, &unmatched).allow,
            "no-match request was allowed in generated case {case}"
        );

        let empty = parse_policy_str(&policy(&[]), "turtle").unwrap();
        assert!(
            !evaluate(&empty, &unmatched).allow,
            "removing all rules did not fail closed in generated case {case}"
        );
    }
}
