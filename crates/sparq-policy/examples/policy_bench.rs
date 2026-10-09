//! Correctness-gated, self-relative policy-evaluation micro-benchmark.
//!
//! [GPT-5.6] sq-gpdwc. The WAC and ACP lanes model their characteristic
//! allow/deny shapes in ODRL so that all lanes exercise the same evaluator.
//! This is not an external comparison or a security/soundness claim.

use sparq_policy::{evaluate, parse_policy_str, Request, ValidatedPolicy, Value, ODRL_NS};
use std::hint::black_box;
use std::time::Instant;

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

struct Case {
    request: Request,
    allow: bool,
}

struct Mix {
    name: &'static str,
    policy: ValidatedPolicy,
    cases: Vec<Case>,
}

fn request(action: &str, target: &str, assignee: &str) -> Request {
    Request::new(format!("{ODRL_NS}{action}"))
        .on(target)
        .by(assignee)
}

fn parse(ttl: &str) -> ValidatedPolicy {
    parse_policy_str(ttl, "turtle").expect("vendored policy fixture must parse")
}

fn mixes() -> Vec<Mix> {
    let wac = parse(&format!(
        "@prefix odrl: <{ODRL_NS}> .\n\
         <urn:policy:wac> a odrl:Set ;\n\
           odrl:permission [ odrl:action odrl:read ; odrl:target <urn:doc:public> ] ;\n\
           odrl:permission [ odrl:action odrl:write ; odrl:target <urn:doc:private> ;\n\
                             odrl:assignee <urn:agent:owner> ] ."
    ));
    let acp = parse(&format!(
        "@prefix odrl: <{ODRL_NS}> .\n\
         <urn:policy:acp> a odrl:Set ;\n\
           odrl:permission [ odrl:action odrl:read ; odrl:target <urn:resource:team> ;\n\
                             odrl:assignee <urn:group:editors> ] ;\n\
           odrl:prohibition [ odrl:action odrl:write ; odrl:target <urn:resource:team> ;\n\
                              odrl:assignee <urn:agent:suspended> ] ."
    ));
    let odrl = parse(&format!(
        "@prefix odrl: <{ODRL_NS}> .\n\
         @prefix xsd: <{XSD}> .\n\
         <urn:policy:odrl> a odrl:Set ;\n\
           odrl:permission [ odrl:action odrl:use ; odrl:target <urn:data:study> ;\n\
             odrl:assignee <urn:agent:researcher> ;\n\
             odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;\n\
                               odrl:rightOperand <urn:purpose:research> ] ;\n\
             odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;\n\
                               odrl:rightOperand \"2030-01-01T00:00:00Z\"^^xsd:dateTime ] ] ."
    ));

    vec![
        Mix {
            name: "wac",
            policy: wac,
            cases: vec![
                Case {
                    request: request("read", "urn:doc:public", "urn:agent:anyone"),
                    allow: true,
                },
                Case {
                    request: request("write", "urn:doc:private", "urn:agent:owner"),
                    allow: true,
                },
                Case {
                    request: request("write", "urn:doc:private", "urn:agent:other"),
                    allow: false,
                },
                Case {
                    request: request("read", "urn:doc:private", "urn:agent:owner"),
                    allow: false,
                },
            ],
        },
        Mix {
            name: "acp",
            policy: acp,
            cases: vec![
                Case {
                    request: request("read", "urn:resource:team", "urn:group:editors"),
                    allow: true,
                },
                Case {
                    request: request("read", "urn:resource:team", "urn:agent:outsider"),
                    allow: false,
                },
                Case {
                    request: request("write", "urn:resource:team", "urn:agent:suspended"),
                    allow: false,
                },
                Case {
                    request: request("read", "urn:resource:other", "urn:group:editors"),
                    allow: false,
                },
            ],
        },
        Mix {
            name: "odrl",
            policy: odrl,
            cases: vec![
                Case {
                    request: odrl_request("urn:purpose:research", "2028-01-01T00:00:00Z"),
                    allow: true,
                },
                Case {
                    request: odrl_request("urn:purpose:marketing", "2028-01-01T00:00:00Z"),
                    allow: false,
                },
                Case {
                    request: odrl_request("urn:purpose:research", "2031-01-01T00:00:00Z"),
                    allow: false,
                },
                Case {
                    request: request("use", "urn:data:study", "urn:agent:other"),
                    allow: false,
                },
            ],
        },
    ]
}

fn odrl_request(purpose: &str, time: &str) -> Request {
    request("use", "urn:data:study", "urn:agent:researcher")
        .with(format!("{ODRL_NS}purpose"), Value::Iri(purpose.into()))
        .with(format!("{ODRL_NS}dateTime"), Value::DateTime(time.into()))
}

fn assert_oracle(mixes: &[Mix]) {
    for mix in mixes {
        for (index, case) in mix.cases.iter().enumerate() {
            let actual = evaluate(&mix.policy, &case.request).allow;
            assert_eq!(actual, case.allow, "{} oracle row {index}", mix.name);
        }
    }
}

fn main() {
    let smoke = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--smoke") => true,
        Some(arg) => panic!("unknown argument {arg:?}; expected --smoke"),
    };
    let mixes = mixes();

    // No timing row may be emitted unless the complete pinned table is green.
    assert_oracle(&mixes);
    let rounds = if smoke { 100 } else { 25_000 };

    println!("=== SPARQ_POLICY_BENCH ===");
    println!(
        "# correctness=green oracle_rows={}",
        mixes.iter().map(|m| m.cases.len()).sum::<usize>()
    );
    println!("# policy-mix\tdecisions\tus");
    for mix in &mixes {
        let start = Instant::now();
        let mut allowed = 0usize;
        for _ in 0..rounds {
            for case in &mix.cases {
                allowed +=
                    usize::from(black_box(evaluate(&mix.policy, black_box(&case.request))).allow);
            }
        }
        black_box(allowed);
        let decisions = rounds * mix.cases.len();
        println!(
            "{}\t{}\t{}",
            mix.name,
            decisions,
            start.elapsed().as_micros()
        );
    }
    println!("=== END SPARQ_POLICY_BENCH ===");
}
