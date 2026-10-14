//! sparq-policy ODRL evaluator micro-benchmark. sq-r06h. Run:
//!     cargo run -p sparq-policy --example bench --release
//!
//! Measures, on a synthetic ODRL `Set` of N permissions (each with an assignee,
//! a target, a dateTime window and a purpose constraint) plus a few prohibitions:
//!   1. policy parse time (RDF → typed model, via the engine's BGP queries);
//!   2. per-request evaluation latency (cold + best-of-K), for an ALLOW request,
//!      a DENY-by-prohibition request, and a DENY-by-constraint request.
//!
//! NON-CANONICAL timing (this box is not a quiet bench instance) — the numbers
//! are a sanity-scale signal, not a published baseline.

use sparq_policy::{evaluate, parse_policy_str, Request, Value};
use std::time::Instant;

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

fn synth_policy(n: usize) -> String {
    let mut s = String::from(
        "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n\
         @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
         <urn:pol/bench> a odrl:Set ;\n",
    );
    for i in 0..n {
        s.push_str(&format!(
            "  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/{i}> ;\n\
             \todrl:assignee <https://party.ex/p{i}> ;\n\
             \todrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;\n\
             \t\todrl:rightOperand \"2030-01-01T00:00:00Z\"^^xsd:dateTime ] ;\n\
             \todrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;\n\
             \t\todrl:rightOperand <urn:purpose/research> ] ] ;\n"
        ));
    }
    // A couple of prohibitions (the override path).
    s.push_str(
        "  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/0> ;\n\
         \todrl:assignee <https://party.ex/blocked> ] .\n",
    );
    s
}

fn timed<T>(label: &str, iters: u32, mut f: impl FnMut() -> T) -> T {
    let mut best = f64::MAX;
    let mut out = None;
    for _ in 0..iters {
        let t = Instant::now();
        out = Some(f());
        best = best.min(t.elapsed().as_secs_f64() * 1e6);
    }
    println!("{label:54} {best:12.2} us");
    out.unwrap()
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(200);
    let ttl = synth_policy(n);
    println!("ODRL Set with {n} permissions + 1 prohibition\n");

    let policy = timed("parse policy (RDF -> typed model)", 3, || {
        parse_policy_str(&ttl, "turtle").expect("parse")
    });
    println!(
        "  parsed {} permissions, {} prohibitions\n",
        policy.permissions.len(),
        policy.prohibitions.len()
    );

    let dt = |s: &str| Value::DateTime(s.to_owned());
    let purpose = || Value::Iri("urn:purpose/research".into());

    // ALLOW: in-window, right purpose, matching party+target.
    let allow = Request::new(format!("{ODRL}read"))
        .on(format!("urn:asset/{}", n / 2))
        .by(format!("https://party.ex/p{}", n / 2))
        .with(format!("{ODRL}dateTime"), dt("2026-06-16T09:00:00Z"))
        .with(format!("{ODRL}purpose"), purpose());
    let d = timed("evaluate ALLOW (last permission matches)", 1000, || {
        evaluate(&policy, &allow)
    });
    assert!(d.allow);

    // DENY by prohibition.
    let denyp = Request::new(format!("{ODRL}read"))
        .on("urn:asset/0")
        .by("https://party.ex/blocked")
        .with(format!("{ODRL}dateTime"), dt("2026-06-16T09:00:00Z"))
        .with(format!("{ODRL}purpose"), purpose());
    let d = timed("evaluate DENY (prohibition overrides)", 1000, || {
        evaluate(&policy, &denyp)
    });
    assert!(!d.allow);

    // DENY by constraint (out of window).
    let denyc = allow
        .clone()
        .with(format!("{ODRL}dateTime"), dt("2031-01-01T00:00:00Z"));
    let d = timed("evaluate DENY (dateTime constraint unmet)", 1000, || {
        evaluate(&policy, &denyc)
    });
    assert!(!d.allow);

    println!("\nok");
}
