//! `odrl:isAnyOf` / `odrl:isNoneOf` set-relation operators — end-to-end parse +
//! evaluate tests through the REAL `parse_policy_str` + `evaluate` path (not a
//! mock). [FABLE-5] sq-uaz85.
//!
//! Per the [ODRL 2.2 operator vocabulary](https://www.w3.org/TR/odrl-vocab/):
//! - `isAnyOf` — the constraint is satisfied when the left-operand value equals
//!   **at least one** member of the right-operand set;
//! - `isNoneOf` — satisfied when it equals **none** of them.
//!
//! The right operand uses the same `|`/space/comma-separated set encoding as
//! `isPartOf`. Load-bearing invariant pinned throughout: evaluation stays
//! **fail-closed** — missing evidence is *unprovable* (never vacuously satisfies
//! `isNoneOf`), an unknown operator never permits, and a numeric/dateTime operand
//! under the negated set test never fails open.

use sparq_policy::{
    contains, evaluate, parse_policy_str, Containment, Operator, Request, Value, ODRL_NS,
};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

fn action(local: &str) -> String {
    format!("{ODRL}{local}")
}

/// A permission gated on `purpose <op> <right>` (TTL helper).
fn purpose_policy(op: &str, right_ttl: &str) -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:{op} ;
                      odrl:rightOperand {right_ttl} ] ] .
"#
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

fn read_x() -> Request {
    Request::new(action("read")).on("urn:asset/x")
}

// ===========================================================================
// Operator parsing (`from_iri`).
// ===========================================================================

/// The two new operator IRIs parse; a plausible-but-unsupported sibling
/// (`odrl:isAllOf` — needs a multi-valued LEFT operand this evaluator does not
/// model) stays `None`, so a constraint carrying it fails closed.
#[test]
fn from_iri_parses_is_any_of_and_is_none_of() {
    assert_eq!(
        Operator::from_iri(&format!("{ODRL_NS}isAnyOf")),
        Some(Operator::IsAnyOf)
    );
    assert_eq!(
        Operator::from_iri(&format!("{ODRL_NS}isNoneOf")),
        Some(Operator::IsNoneOf)
    );
    assert_eq!(Operator::from_iri(&format!("{ODRL_NS}isAllOf")), None);
}

// ===========================================================================
// isAnyOf — satisfied iff the value is IN the right-operand set.
// ===========================================================================

/// `purpose isAnyOf "research|education"`: a listed purpose grants, an unlisted
/// one is a definite mismatch (deny).
#[test]
fn is_any_of_member_grants_non_member_denied() {
    let p = purpose_policy("isAnyOf", r#""urn:purpose/research|urn:purpose/education""#);
    let member = read_x().for_purpose(Value::Iri("urn:purpose/research".into()));
    assert!(evaluate(&p, &member).allow, "listed purpose must grant");
    let other_member = read_x().for_purpose(Value::Iri("urn:purpose/education".into()));
    assert!(
        evaluate(&p, &other_member).allow,
        "every listed member must grant"
    );
    let non_member = read_x().for_purpose(Value::Iri("urn:purpose/marketing".into()));
    assert!(
        !evaluate(&p, &non_member).allow,
        "unlisted purpose must be denied"
    );
}

/// The PSS NOW-mandate shape: a `recipient isAnyOf <list>` permission gates on who
/// is asking (the requesting party doubles as the recipient-of-data).
#[test]
fn recipient_is_any_of_list_gates_on_requester() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/r> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:isAnyOf ;
                      odrl:rightOperand "https://alice.ex/me|https://carol.ex/me" ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert!(evaluate(&p, &read_x().by("https://alice.ex/me")).allow);
    assert!(evaluate(&p, &read_x().by("https://carol.ex/me")).allow);
    assert!(!evaluate(&p, &read_x().by("https://eve.ex/me")).allow);
    // No identity at all → unprovable → fail-closed.
    assert!(!evaluate(&p, &read_x()).allow);
}

/// An empty right-operand set can never be satisfied by `isAnyOf` (no member to
/// equal) — fail-closed.
#[test]
fn is_any_of_empty_set_never_satisfied() {
    let p = purpose_policy("isAnyOf", r#""""#);
    let req = read_x().for_purpose(Value::Iri("urn:purpose/research".into()));
    assert!(!evaluate(&p, &req).allow, "isAnyOf over the empty set is unsatisfiable");
}

// ===========================================================================
// isNoneOf — satisfied iff the value is NOT in the right-operand set.
// ===========================================================================

/// `purpose isNoneOf "marketing|profiling"`: an unlisted purpose grants, a listed
/// (excluded) one is denied.
#[test]
fn is_none_of_non_member_grants_member_denied() {
    let p = purpose_policy(
        "isNoneOf",
        r#""urn:purpose/marketing|urn:purpose/profiling""#,
    );
    let outside = read_x().for_purpose(Value::Iri("urn:purpose/research".into()));
    assert!(evaluate(&p, &outside).allow, "unlisted purpose must grant");
    let excluded = read_x().for_purpose(Value::Iri("urn:purpose/marketing".into()));
    assert!(
        !evaluate(&p, &excluded).allow,
        "a listed (excluded) purpose must be denied"
    );
    let excluded2 = read_x().for_purpose(Value::Iri("urn:purpose/profiling".into()));
    assert!(!evaluate(&p, &excluded2).allow);
}

/// `isNoneOf` over an **empty** set is vacuously satisfied for a stated
/// string/IRI value (nothing is excluded — the dual of `is_any_of_empty_set`),
/// but ONLY with evidence present: see `missing_evidence_fails_closed_for_both`.
#[test]
fn is_none_of_empty_set_vacuously_satisfied_with_evidence() {
    let p = purpose_policy("isNoneOf", r#""""#);
    let req = read_x().for_purpose(Value::Iri("urn:purpose/research".into()));
    assert!(
        evaluate(&p, &req).allow,
        "isNoneOf over the empty set excludes nothing"
    );
}

/// **Load-bearing fail-closed invariant:** a request with NO evidence for the
/// dimension is *unprovable* for BOTH operators — in particular `isNoneOf` is
/// never vacuously satisfied by an unknown value ("we don't know who/what it is"
/// is not "it is none of them").
#[test]
fn missing_evidence_fails_closed_for_both() {
    let any = purpose_policy("isAnyOf", r#""urn:purpose/research""#);
    let none = purpose_policy("isNoneOf", r#""urn:purpose/marketing""#);
    let no_evidence = read_x(); // no purpose stated
    assert!(!evaluate(&any, &no_evidence).allow, "isAnyOf: unprovable ⇒ deny");
    assert!(
        !evaluate(&none, &no_evidence).allow,
        "isNoneOf: unprovable ⇒ deny (never vacuously satisfied)"
    );
}

/// A prohibition under `isNoneOf` carves out every purpose OUTSIDE the set: the
/// listed purpose is the only one not carved out.
#[test]
fn is_none_of_prohibition_carves_out_non_members() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
    odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
    odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
        odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isNoneOf ;
                          odrl:rightOperand "urn:purpose/research" ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // A non-research purpose satisfies the prohibition's isNoneOf → carved out.
    let marketing = read_x().for_purpose(Value::Iri("urn:purpose/marketing".into()));
    assert!(!evaluate(&p, &marketing).allow);
    // The research purpose fails the prohibition's isNoneOf → not carved out → the
    // unconstrained permission grants.
    let research = read_x().for_purpose(Value::Iri("urn:purpose/research".into()));
    assert!(evaluate(&p, &research).allow);
}

/// A numeric or dateTime operand under `isNoneOf` never fails OPEN on the lexical
/// representation gap (`Value::Num::as_str()` is `""`; dateTime set membership is
/// lexical, not instant-normalized): the constraint is simply not satisfied.
#[test]
fn is_none_of_numeric_and_datetime_operands_fail_closed() {
    // Numeric right-operand: the "set" has no faithful lexical form → never satisfied,
    // even though the actual value (3) is genuinely different from the bound (5).
    let num_bound = purpose_policy("isNoneOf", r#""5"^^<http://www.w3.org/2001/XMLSchema#integer>"#);
    let num_req = read_x().for_purpose(Value::Num(3.0));
    assert!(
        !evaluate(&num_bound, &num_req).allow,
        "numeric isNoneOf must fail closed, never vacuously grant"
    );
    // Numeric actual against a string set: as_str() is "" → not representable.
    let str_bound = purpose_policy("isNoneOf", r#""1|2""#);
    assert!(!evaluate(&str_bound, &read_x().for_purpose(Value::Num(3.0))).allow);
    // dateTime actual: lexical set membership can't see instant equivalence
    // (13:00+02:00 IS 11:00Z), so the negation is not faithful → fail-closed.
    let dt_bound = purpose_policy("isNoneOf", r#""2026-06-16T11:00:00Z""#);
    let dt_req = read_x().for_purpose(Value::DateTime("2026-06-16T13:00:00+02:00".into()));
    assert!(
        !evaluate(&dt_bound, &dt_req).allow,
        "dateTime isNoneOf must fail closed on the lexical/instant gap"
    );
}

/// An unknown operator IRI still fails closed end-to-end: `odrl:isAllOf` (not
/// implemented — it needs a multi-valued left operand) parses to the
/// unsatisfiable guard, so the permission never grants.
#[test]
fn unknown_operator_is_all_of_fails_closed_end_to_end() {
    let p = purpose_policy("isAllOf", r#""urn:purpose/research""#);
    let req = read_x().for_purpose(Value::Iri("urn:purpose/research".into()));
    assert!(
        !evaluate(&p, &req).allow,
        "an unrecognized operator must never permit"
    );
}

// ===========================================================================
// Taxonomic subsumption composition (purpose taxonomy × the new operators).
// ===========================================================================

/// With a purpose taxonomy supplied, `isAnyOf` matches a stated purpose that is
/// transitively NARROWER than a listed member (`clinical ⊑ research`), exactly
/// like `isPartOf`; and `isNoneOf` also excludes such a sub-purpose (a
/// sub-purpose IS the excluded purpose — the carve-out is not widened away).
#[test]
fn subsumption_composes_with_is_any_of_and_is_none_of() {
    let any = purpose_policy("isAnyOf", r#""urn:p/research|urn:p/education""#);
    let clinical_any = read_x()
        .for_purpose(Value::Iri("urn:p/research/clinical".into()))
        .with_purpose_subsumption("urn:p/research/clinical", "urn:p/research");
    assert!(
        evaluate(&any, &clinical_any).allow,
        "a sub-purpose of a listed member matches isAnyOf under the supplied taxonomy"
    );

    let none = purpose_policy("isNoneOf", r#""urn:p/research""#);
    let clinical_none = read_x()
        .for_purpose(Value::Iri("urn:p/research/clinical".into()))
        .with_purpose_subsumption("urn:p/research/clinical", "urn:p/research");
    assert!(
        !evaluate(&none, &clinical_none).allow,
        "a sub-purpose of an excluded member is also excluded (no widening)"
    );
    // An unrelated purpose (no ⊑ path into the excluded set) still grants.
    let unrelated = read_x()
        .for_purpose(Value::Iri("urn:p/education".into()))
        .with_purpose_subsumption("urn:p/research/clinical", "urn:p/research");
    assert!(evaluate(&none, &unrelated).allow);
}

// ===========================================================================
// Static containment analysis admits the new operators (outer_admits_value).
// ===========================================================================

/// An inner `purpose eq a` is contained by an outer `purpose isAnyOf "a|b"` offer
/// (the outer set admits the pinned inner value); an inner value OUTSIDE the outer
/// set is not proven contained.
#[test]
fn containment_admits_inner_eq_under_outer_is_any_of() {
    let outer = purpose_policy("isAnyOf", r#""urn:p/a|urn:p/b""#);
    let inner_in = purpose_policy("eq", "<urn:p/a>");
    assert_eq!(contains(&outer, &inner_in), Containment::Contains);
    let inner_out = purpose_policy("eq", "<urn:p/c>");
    assert_ne!(
        contains(&outer, &inner_out),
        Containment::Contains,
        "a value outside the outer isAnyOf set must never be claimed contained"
    );
}

/// An inner `purpose eq c` (outside the excluded set) is contained by an outer
/// `purpose isNoneOf "a|b"` offer; an inner value INSIDE the excluded set is not.
#[test]
fn containment_admits_inner_eq_under_outer_is_none_of() {
    let outer = purpose_policy("isNoneOf", r#""urn:p/a|urn:p/b""#);
    let inner_out = purpose_policy("eq", "<urn:p/c>");
    assert_eq!(contains(&outer, &inner_out), Containment::Contains);
    let inner_in = purpose_policy("eq", "<urn:p/a>");
    assert_ne!(
        contains(&outer, &inner_in),
        Containment::Contains,
        "a value inside the outer isNoneOf exclusion must never be claimed contained"
    );
}
