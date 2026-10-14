//! Default-feature (`count-enforcement` OFF) `odrl:count` evaluation tests.
//! sq-bif.7.
//!
//! `odrl:count` enforcement comes in two layers (see crates/sparq-policy/src/lib.rs):
//!   - DEFAULT (this file): the *stateless* numeric constraint it always was — the
//!     caller supplies the actual count in the request context and `evaluate` compares
//!     it against the constraint's `rightOperand` under `lt`/`lteq`/`eq`/`gt`/`gteq`.
//!     NO counter store, NO state, NO `count-enforcement` feature.
//!   - `count-enforcement` (tests/odrl_count*.rs, `#![cfg(feature = "count-enforcement")]`):
//!     the *stateful* counter-store path (`evaluate_and_exercise`) that increments per
//!     use and denies once the limit is reached.
//!
//! This file is deliberately NOT feature-gated: it is the regression guard for the bead's
//! "no compile-out regression" requirement — proving the stateless `odrl:count` path keeps
//! evaluating correctly in the DEFAULT build, where the entire `count`/`count_file`/
//! `count_backend` surface is compiled out. Every assertion fails if that path regresses.
//!
//! It complements `odrl_eval.rs::count_numeric_operators` (which covers `lteq`) by
//! exercising the FULL operator matrix (`lt`/`lteq`/`eq`/`gt`/`gteq`), the boundary cases,
//! the missing-count fail-closed default, and the prohibition-side count carve-out.

use sparq_policy::{evaluate, parse_policy_str, Request, Value};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

fn left(local: &str) -> String {
    format!("{ODRL}{local}")
}

/// A `odrl:Set` permission on `odrl:use`/`urn:asset/x` with one `odrl:count`
/// constraint under `operator` against the integer `bound`.
fn count_policy_ttl(operator_local: &str, bound: u32) -> String {
    format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/c> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:count ;
                      odrl:operator odrl:{operator_local} ;
                      odrl:rightOperand "{bound}"^^xsd:integer ] ] .
"#
    )
}

fn req_with_count(n: f64) -> Request {
    Request::new(left("use"))
        .on("urn:asset/x")
        .with(left("count"), Value::Num(n))
}

/// `odrl:count lteq 5`: counts 0..=5 ALLOW, 6 DENY. (Boundary at the bound itself.)
#[test]
fn stateless_lteq_allows_up_to_and_including_bound() {
    let p = parse_policy_str(&count_policy_ttl("lteq", 5), "turtle").unwrap();
    assert!(evaluate(&p, &req_with_count(0.0)).allow, "0 lteq 5 must ALLOW");
    assert!(evaluate(&p, &req_with_count(5.0)).allow, "5 lteq 5 must ALLOW");
    let d = evaluate(&p, &req_with_count(6.0));
    assert!(!d.allow, "6 lteq 5 must DENY");
    assert!(
        d.unmet_constraints.iter().any(|m| m.contains("count")),
        "DENY must cite the count constraint: {d:?}"
    );
}

/// `odrl:count lt 5`: 4 ALLOW, 5 DENY (strict — the bound itself is excluded).
#[test]
fn stateless_lt_excludes_the_bound() {
    let p = parse_policy_str(&count_policy_ttl("lt", 5), "turtle").unwrap();
    assert!(evaluate(&p, &req_with_count(4.0)).allow, "4 lt 5 must ALLOW");
    assert!(!evaluate(&p, &req_with_count(5.0)).allow, "5 lt 5 must DENY");
    assert!(!evaluate(&p, &req_with_count(6.0)).allow, "6 lt 5 must DENY");
}

/// `odrl:count eq 3`: only exactly 3 ALLOWs; 2 and 4 DENY.
#[test]
fn stateless_eq_matches_only_the_bound() {
    let p = parse_policy_str(&count_policy_ttl("eq", 3), "turtle").unwrap();
    assert!(!evaluate(&p, &req_with_count(2.0)).allow, "2 eq 3 must DENY");
    assert!(evaluate(&p, &req_with_count(3.0)).allow, "3 eq 3 must ALLOW");
    assert!(!evaluate(&p, &req_with_count(4.0)).allow, "4 eq 3 must DENY");
}

/// `odrl:count gt 2`: 2 DENY (strict), 3 ALLOW.
#[test]
fn stateless_gt_excludes_the_bound() {
    let p = parse_policy_str(&count_policy_ttl("gt", 2), "turtle").unwrap();
    assert!(!evaluate(&p, &req_with_count(2.0)).allow, "2 gt 2 must DENY");
    assert!(evaluate(&p, &req_with_count(3.0)).allow, "3 gt 2 must ALLOW");
}

/// `odrl:count gteq 2`: 1 DENY, 2 ALLOW (bound included), 3 ALLOW.
#[test]
fn stateless_gteq_includes_the_bound() {
    let p = parse_policy_str(&count_policy_ttl("gteq", 2), "turtle").unwrap();
    assert!(!evaluate(&p, &req_with_count(1.0)).allow, "1 gteq 2 must DENY");
    assert!(evaluate(&p, &req_with_count(2.0)).allow, "2 gteq 2 must ALLOW");
    assert!(evaluate(&p, &req_with_count(3.0)).allow, "3 gteq 2 must ALLOW");
}

/// FAIL-CLOSED: a `odrl:count` constraint with NO count value in the request context is
/// unsatisfied (we have no evidence the limit is met), so the permission does NOT grant.
/// This is the core stateless-default property: without the counter store, the count is
/// the caller's responsibility, and a missing count cannot widen access.
#[test]
fn stateless_missing_count_fails_closed() {
    let p = parse_policy_str(&count_policy_ttl("lteq", 5), "turtle").unwrap();
    // Same action/target, but NO `odrl:count` value supplied.
    let no_count = Request::new(left("use")).on("urn:asset/x");
    let d = evaluate(&p, &no_count);
    assert!(
        !d.allow,
        "a count constraint with no supplied count must fail closed: {d:?}"
    );
    assert!(
        d.unmet_constraints.iter().any(|m| m.contains("count")),
        "the unmet count constraint must be reported: {d:?}"
    );
}

/// A `odrl:count` carve-out on the PROHIBITION side: the prohibition forbids `use`
/// once the count exceeds the bound. Under deny-overrides, a request whose count
/// triggers the prohibition is DENIED even though the permission would otherwise grant;
/// a request below it is ALLOWED. Proves stateless count drives both rule kinds.
#[test]
fn stateless_count_on_prohibition_denies_over_bound() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/p> a odrl:Set ;
    odrl:permission [ odrl:action odrl:use ; odrl:target <urn:asset/x> ] ;
    odrl:prohibition [ odrl:action odrl:use ; odrl:target <urn:asset/x> ;
        odrl:constraint [ odrl:leftOperand odrl:count ;
                          odrl:operator odrl:gt ;
                          odrl:rightOperand "10"^^xsd:integer ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    assert_eq!(p.permissions.len(), 1, "expected one permission: {p:?}");
    assert_eq!(p.prohibitions.len(), 1, "expected one prohibition: {p:?}");
    // count 5 (not > 10): prohibition does NOT fire → permission stands → ALLOW.
    assert!(
        evaluate(&p, &req_with_count(5.0)).allow,
        "count 5 must not trigger the >10 prohibition"
    );
    // count 11 (> 10): prohibition fires → deny-overrides → DENY.
    let d = evaluate(&p, &req_with_count(11.0));
    assert!(
        !d.allow,
        "count 11 (>10) must trigger the prohibition and DENY: {d:?}"
    );
}
