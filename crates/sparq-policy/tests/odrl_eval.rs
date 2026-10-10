//! End-to-end ODRL parse + evaluate tests. [OPUS-4.8] sq-r06h.
//!
//! Coverage: permission grant, prohibition-overrides-permission, constraint
//! gating (inside/outside time window; purpose eq/neq; recipient isPartOf;
//! count gt/lteq), duty-not-discharged, fail-closed defaults (empty policy,
//! unknown action/target/party, malformed constraint, unknown operator).
//!
//! Policies are hand-authored in Turtle following the W3C ODRL 2.2 examples
//! (the public test-suite examples are equivalent shapes: a `odrl:Set` with a
//! blank-node permission/prohibition carrying action/target/assignee and
//! blank-node constraints).

use sparq_policy::{
    datetime_status, evaluate, parse_policy_str, prohibition_status, purpose_status,
    recipient_status, DateTimeMatch, Decision, ProhibitionStatus, PurposeMatch, RecipientMatch,
    Request, Value,
};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
const XSD_DT: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const XSD_DT_NS: &str = "http://www.w3.org/2001/XMLSchema#";

fn dt(s: &str) -> Value {
    Value::DateTime(s.to_owned())
}
fn left(local: &str) -> String {
    format!("{ODRL}{local}")
}

// ---------------------------------------------------------------------------
// 1. Permission grant — a bare permission with a matching action/target/party.
// ---------------------------------------------------------------------------
#[test]
fn permission_grants() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/grant> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <urn:asset/x> ;
    odrl:assignee <https://alice.ex/me> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert_eq!(p.permissions.len(), 1);
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .by("https://alice.ex/me");
    let d = evaluate(&p, &req);
    assert!(d.allow, "{d:?}");
    assert_eq!(d.matched_rules.len(), 1);
}

#[test]
fn wrong_party_denied_fail_closed() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/grant> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:assignee <https://alice.ex/me> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // Mallory is not the assignee → no permission matches → DENY.
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .by("https://mallory.ex/me");
    assert!(!evaluate(&p, &req).allow);
    // Wrong target → DENY.
    let req2 = Request::new(left("read"))
        .on("urn:asset/secret")
        .by("https://alice.ex/me");
    assert!(!evaluate(&p, &req2).allow);
    // Wrong action → DENY.
    let req3 = Request::new(left("write"))
        .on("urn:asset/x")
        .by("https://alice.ex/me");
    assert!(!evaluate(&p, &req3).allow);
}

#[test]
fn use_umbrella_action_subsumes() {
    // odrl:use is the umbrella action — a permission for `use` permits `read`.
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/u> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let req = Request::new(left("read")).on("urn:asset/x");
    assert!(evaluate(&p, &req).allow);
}

// ---------------------------------------------------------------------------
// 2. Prohibition overrides permission.
// ---------------------------------------------------------------------------
#[test]
fn prohibition_overrides_permission() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/po> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
                     odrl:assignee <https://mallory.ex/me> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // Alice: permission applies, no prohibition matches her → ALLOW.
    let alice = Request::new(left("read"))
        .on("urn:asset/x")
        .by("https://alice.ex/me");
    assert!(evaluate(&p, &alice).allow);
    // Mallory: the prohibition matches → DENY even though the permission would.
    let mallory = Request::new(left("read"))
        .on("urn:asset/x")
        .by("https://mallory.ex/me");
    let d = evaluate(&p, &mallory);
    assert!(!d.allow);
    assert_eq!(
        d.matched_rules.len(),
        1,
        "the prohibition is the justification"
    );
}

// ---------------------------------------------------------------------------
// 3. Constraint gating — dateTime window (inside ALLOW, outside DENY).
// ---------------------------------------------------------------------------
#[test]
fn datetime_window_gates() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/t> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ;
                      odrl:operator odrl:lteq ;
                      odrl:rightOperand "2026-12-31T23:59:59Z"^^xsd:dateTime ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let base = Request::new(left("read")).on("urn:asset/x");
    // Inside the window → ALLOW.
    assert!(
        evaluate(
            &p,
            &base
                .clone()
                .with(left("dateTime"), dt("2026-06-16T09:00:00Z"))
        )
        .allow
    );
    // After the window → DENY.
    assert!(
        !evaluate(
            &p,
            &base
                .clone()
                .with(left("dateTime"), dt("2027-03-01T00:00:00Z"))
        )
        .allow
    );
    // No dateTime supplied at all → DENY (fail-closed: no evidence).
    assert!(!evaluate(&p, &base).allow);
}

#[test]
fn purpose_eq_and_neq() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ;
                      odrl:operator odrl:eq ;
                      odrl:rightOperand <urn:purpose/research> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("use")).on("urn:asset/x");
    // Matching purpose → ALLOW.
    let ok = base
        .clone()
        .with(left("purpose"), Value::Iri("urn:purpose/research".into()));
    assert!(evaluate(&p, &ok).allow);
    // Different purpose → DENY.
    let bad = base.with(left("purpose"), Value::Iri("urn:purpose/marketing".into()));
    assert!(!evaluate(&p, &bad).allow);
}

#[test]
fn recipient_is_part_of_set() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/r> a odrl:Set ; odrl:permission [
    odrl:action odrl:distribute ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ;
                      odrl:operator odrl:isPartOf ;
                      odrl:rightOperand "nodeB|nodeC" ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("distribute")).on("urn:asset/x");
    assert!(
        evaluate(
            &p,
            &base
                .clone()
                .with(left("recipient"), Value::Str("nodeB".into()))
        )
        .allow
    );
    assert!(
        evaluate(
            &p,
            &base
                .clone()
                .with(left("recipient"), Value::Str("nodeC".into()))
        )
        .allow
    );
    // Not in the permitted recipient set → DENY.
    assert!(
        !evaluate(
            &p,
            &base.with(left("recipient"), Value::Str("nodeD".into()))
        )
        .allow
    );
}

#[test]
fn count_numeric_operators() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/c> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:count ;
                      odrl:operator odrl:lteq ;
                      odrl:rightOperand "5"^^xsd:integer ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("use")).on("urn:asset/x");
    assert!(evaluate(&p, &base.clone().with(left("count"), Value::Num(3.0))).allow);
    assert!(evaluate(&p, &base.clone().with(left("count"), Value::Num(5.0))).allow);
    assert!(!evaluate(&p, &base.with(left("count"), Value::Num(6.0))).allow);
}

// ---------------------------------------------------------------------------
// 4. Duty not discharged → DENY; discharged → ALLOW.
// ---------------------------------------------------------------------------
#[test]
fn duty_must_be_discharged() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/d> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:duty [ odrl:action odrl:anonymize ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    assert_eq!(p.permissions[0].duties.len(), 1);
    let base = Request::new(left("read")).on("urn:asset/x");
    // Duty not discharged → DENY.
    let d = evaluate(&p, &base);
    assert!(!d.allow);
    assert!(
        d.unmet_constraints.iter().any(|m| m.contains("duty")),
        "{d:?}"
    );
    // Discharged → ALLOW.
    let ok = base.discharge(left("anonymize"));
    assert!(evaluate(&p, &ok).allow);
}

// ---------------------------------------------------------------------------
// 5. Fail-closed defaults.
// ---------------------------------------------------------------------------
#[test]
fn empty_policy_denies_everything() {
    let p = parse_policy_str("@prefix odrl: <http://www.w3.org/ns/odrl/2/> .", "turtle").unwrap();
    let d: Decision = evaluate(&p, &Request::new(left("read")).on("urn:asset/x"));
    assert!(!d.allow);
    assert!(d.matched_rules.is_empty());
}

#[test]
fn unknown_operator_fails_closed() {
    // odrl:hasPart is not a comparison operator we support → the constraint
    // becomes an unsatisfiable guard, so the permission can never match.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/x> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ;
                      odrl:operator odrl:hasPart ;
                      odrl:rightOperand <urn:purpose/research> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .with(left("purpose"), Value::Iri("urn:purpose/research".into()));
    assert!(
        !evaluate(&p, &req).allow,
        "unknown operator must fail closed"
    );
}

#[test]
fn malformed_constraint_fails_closed() {
    // A constraint missing its operator/rightOperand is an unsatisfiable guard.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/m> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("read")).on("urn:asset/x");
    assert!(!evaluate(&p, &req).allow);
}

#[test]
fn untyped_policy_with_rules_is_parsed() {
    // No `a odrl:Set` — the parser still finds the rule via odrl:permission.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/untyped> odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    assert_eq!(p.iri.as_deref(), Some("urn:pol/untyped"));
    assert!(evaluate(&p, &Request::new(left("read")).on("urn:asset/x")).allow);
}

#[test]
fn datetime_xsd_typed_constant_roundtrips() {
    // Make sure the XSD dateTime datatype is recognized (parses to Value::DateTime,
    // compared by instant not string-coincidence).
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/t2> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ;
                      odrl:operator odrl:gteq ;
                      odrl:rightOperand "2026-01-01T00:00:00Z"^^<{XSD_DT}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("read")).on("urn:asset/x");
    // At/after the lower bound → ALLOW.
    assert!(
        evaluate(
            &p,
            &base
                .clone()
                .with(left("dateTime"), dt("2026-06-16T00:00:00Z"))
        )
        .allow
    );
    // Before the lower bound → DENY.
    assert!(!evaluate(&p, &base.with(left("dateTime"), dt("2025-12-31T00:00:00Z"))).allow);
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-2pcf — prohibition_status: the three-valued deny-retraction dual
// of matched_prohibition (Applies / Ambiguous / Withdrawn). The bridge uses this to
// retract a materialized deny ONLY on a DEFINITE withdrawal, never on missing evidence.
// ---------------------------------------------------------------------------
fn windowed_prohibition() -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/p> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:write ; odrl:target <urn:asset/x> ;
    odrl:assignee <https://alice.ex/me> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
        odrl:rightOperand "2026-01-01T00:00:00Z"^^<{XSD_DT}> ] ] .
"#
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

#[test]
fn prohibition_status_applies_when_constraint_holds() {
    let p = windowed_prohibition();
    // Evidence the window holds (now < bound) → still carves out → KEEP the deny.
    let req = Request::new(left("write"))
        .on("urn:asset/x")
        .by("https://alice.ex/me")
        .with(left("dateTime"), dt("2025-06-01T00:00:00Z"));
    assert_eq!(prohibition_status(&p, &req), ProhibitionStatus::Applies);
}

#[test]
fn prohibition_status_withdrawn_when_constraint_definitely_false() {
    let p = windowed_prohibition();
    // Evidence the window LAPSED (now >= bound, operator lt) → definitely no → RETRACT.
    let req = Request::new(left("write"))
        .on("urn:asset/x")
        .by("https://alice.ex/me")
        .with(left("dateTime"), dt("2026-06-01T00:00:00Z"));
    assert_eq!(prohibition_status(&p, &req), ProhibitionStatus::Withdrawn);
}

#[test]
fn prohibition_status_ambiguous_when_no_evidence() {
    let p = windowed_prohibition();
    // NO dateTime evidence → cannot prove the window lapsed → AMBIGUOUS → KEEP the deny.
    let req = Request::new(left("write"))
        .on("urn:asset/x")
        .by("https://alice.ex/me");
    assert_eq!(prohibition_status(&p, &req), ProhibitionStatus::Ambiguous);
}

#[test]
fn prohibition_status_withdrawn_on_structural_mismatch() {
    let p = windowed_prohibition();
    // A different party is not carved out at all — structurally Withdrawn, even with
    // NO constraint evidence (a structural attribute is a DEFINITE non-match).
    let other = Request::new(left("write"))
        .on("urn:asset/x")
        .by("https://bob.ex/me");
    assert_eq!(prohibition_status(&p, &other), ProhibitionStatus::Withdrawn);
    // An empty policy (prohibition removed) is also Withdrawn.
    let empty = parse_policy_str(
        &format!(r#"@prefix odrl: <{ODRL}> . <urn:pol/p> a odrl:Set ."#),
        "turtle",
    )
    .unwrap();
    let req = Request::new(left("write"))
        .on("urn:asset/x")
        .by("https://alice.ex/me");
    assert_eq!(
        prohibition_status(&empty, &req),
        ProhibitionStatus::Withdrawn
    );
}

#[test]
fn prohibition_status_ambiguous_only_if_no_other_match() {
    // Two prohibitions: one unconstrained (always matches) + one windowed (ambiguous w/o
    // evidence). The unconstrained one still matches → Applies wins over Ambiguous.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/p> a odrl:Set ;
    odrl:prohibition [ odrl:action odrl:write ; odrl:target <urn:asset/x> ;
        odrl:assignee <https://alice.ex/me> ] ;
    odrl:prohibition [ odrl:action odrl:write ; odrl:target <urn:asset/x> ;
        odrl:assignee <https://alice.ex/me> ;
        odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
            odrl:rightOperand "2026-01-01T00:00:00Z"^^<{XSD_DT}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("write"))
        .on("urn:asset/x")
        .by("https://alice.ex/me");
    assert_eq!(prohibition_status(&p, &req), ProhibitionStatus::Applies);
}

// ===========================================================================
// [OPUS-4.8] sq-q56r — faithful odrl:purpose enforcement: a purpose constraint
// grants ONLY when the request carries a matching purpose; a missing purpose is
// Unprovable → fail-closed (permission does not grant; prohibition not withdrawn);
// a mismatch is a definite no. Match is EXACT (no hierarchy/subsumption). The
// `purpose_status` helper REPORTS exactly what `evaluate` checks (the honesty point).
// ===========================================================================

const RESEARCH: &str = "urn:purpose/research";
const MARKETING: &str = "urn:purpose/marketing";

/// alice MAY use asset-X, gated on purpose = research (exact IRI).
fn purpose_permission() -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/purp> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ; odrl:assignee <https://alice.ex/me> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
                      odrl:rightOperand <{RESEARCH}> ] ] .
"#
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

#[test]
fn purpose_match_grants() {
    let p = purpose_permission();
    let req = Request::new(left("use"))
        .on("urn:asset/x")
        .by("https://alice.ex/me")
        .for_purpose(Value::Iri(RESEARCH.into()));
    assert!(evaluate(&p, &req).allow, "matching purpose grants");
    assert_eq!(
        purpose_status(&p.permissions[0], &req),
        PurposeMatch::Satisfied
    );
    // The request's purpose is auditable as exactly what it stated.
    assert_eq!(req.purpose().map(Value::as_str), Some(RESEARCH));
}

#[test]
fn purpose_mismatch_denies() {
    let p = purpose_permission();
    let req = Request::new(left("use"))
        .on("urn:asset/x")
        .by("https://alice.ex/me")
        .for_purpose(Value::Iri(MARKETING.into()));
    assert!(
        !evaluate(&p, &req).allow,
        "a different purpose must not grant"
    );
    assert_eq!(
        purpose_status(&p.permissions[0], &req),
        PurposeMatch::DefinitelyUnsatisfied
    );
}

#[test]
fn missing_purpose_fails_closed() {
    // THE honesty test: no purpose stated → Unprovable → permission does NOT grant.
    // "No purpose stated" is never treated as "any purpose allowed".
    let p = purpose_permission();
    let no_purpose = Request::new(left("use"))
        .on("urn:asset/x")
        .by("https://alice.ex/me");
    assert!(
        !evaluate(&p, &no_purpose).allow,
        "missing purpose must fail closed (not grant)"
    );
    assert_eq!(no_purpose.purpose(), None, "no purpose evidence supplied");
    assert_eq!(
        purpose_status(&p.permissions[0], &no_purpose),
        PurposeMatch::Unprovable
    );
}

#[test]
fn purpose_match_is_exact_no_hierarchy() {
    // Match is EXACT — a narrower/broader purpose IRI is NOT subsumed. Documents the
    // boundary so the helper never over-claims hierarchy matching it does not perform.
    let p = purpose_permission(); // gated on <urn:purpose/research>
    let subpurpose = Request::new(left("use"))
        .on("urn:asset/x")
        .by("https://alice.ex/me")
        // a plausibly-narrower purpose IRI — NOT matched (no subsumption).
        .for_purpose(Value::Iri("urn:purpose/research/clinical".into()));
    assert!(
        !evaluate(&p, &subpurpose).allow,
        "exact-match only: a sub-purpose IRI is not subsumed"
    );
    assert_eq!(
        purpose_status(&p.permissions[0], &subpurpose),
        PurposeMatch::DefinitelyUnsatisfied
    );
}

#[test]
fn purpose_set_is_part_of() {
    // An explicit isPartOf purpose set is honoured (membership), but it is still the
    // exact set the constraint names — not a hierarchy.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/ps> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand "{RESEARCH}|{MARKETING}" ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("use")).on("urn:asset/x");
    assert!(evaluate(&p, &base.clone().for_purpose(Value::Str(RESEARCH.into()))).allow);
    assert!(evaluate(&p, &base.clone().for_purpose(Value::Str(MARKETING.into()))).allow);
    // Outside the set → deny; missing → Unprovable (fail-closed).
    assert!(
        !evaluate(
            &p,
            &base
                .clone()
                .for_purpose(Value::Str("urn:purpose/ads".into()))
        )
        .allow
    );
    assert!(!evaluate(&p, &base).allow);
}

#[test]
fn purpose_neq_constraint() {
    // odrl:neq purpose: granted for ANY purpose except the named one — but a stated
    // purpose is still REQUIRED (missing → Unprovable → fail-closed).
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/pn> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:neq ;
                      odrl:rightOperand <{MARKETING}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("use")).on("urn:asset/x");
    assert!(evaluate(&p, &base.clone().for_purpose(Value::Iri(RESEARCH.into()))).allow);
    assert!(!evaluate(&p, &base.clone().for_purpose(Value::Iri(MARKETING.into()))).allow);
    // Missing purpose is unprovable, NOT "any purpose ≠ marketing" → fail-closed.
    assert!(
        !evaluate(&p, &base).allow,
        "neq purpose still needs evidence"
    );
}

#[test]
fn purpose_prohibition_dual() {
    // The dual: a prohibition gated on purpose carves the request out ONLY when the
    // stated purpose matches; a different purpose does NOT carve out; a MISSING purpose
    // keeps the carve-out ambiguous (the deny is NOT withdrawn — fail-closed).
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/pp> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ; odrl:assignee <https://alice.ex/me> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
                      odrl:rightOperand <{MARKETING}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let prohib = &p.prohibitions[0];
    let base = Request::new(left("use"))
        .on("urn:asset/x")
        .by("https://alice.ex/me");

    // Stated marketing purpose → the prohibition applies (carve-out).
    let marketing = base.clone().for_purpose(Value::Iri(MARKETING.into()));
    assert_eq!(purpose_status(prohib, &marketing), PurposeMatch::Satisfied);
    assert_eq!(
        prohibition_status(&p, &marketing),
        ProhibitionStatus::Applies
    );

    // Stated a DIFFERENT purpose → the prohibition definitely no longer carves THIS out.
    let research = base.clone().for_purpose(Value::Iri(RESEARCH.into()));
    assert_eq!(
        purpose_status(prohib, &research),
        PurposeMatch::DefinitelyUnsatisfied
    );
    assert_eq!(
        prohibition_status(&p, &research),
        ProhibitionStatus::Withdrawn
    );

    // NO purpose stated → ambiguous: the deny is NOT withdrawn (fail-closed).
    assert_eq!(purpose_status(prohib, &base), PurposeMatch::Unprovable);
    assert_eq!(prohibition_status(&p, &base), ProhibitionStatus::Ambiguous);
}

#[test]
fn purpose_not_constrained_when_absent() {
    // A rule with no purpose constraint reports NotConstrained (purpose places no bound).
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/np> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("read")).on("urn:asset/x");
    assert_eq!(
        purpose_status(&p.permissions[0], &req),
        PurposeMatch::NotConstrained
    );
}

// ===========================================================================
// [OPUS-4.8] sq-5037 — odrl:recipient `neq` / "everyone-except-X": a permission
// (or prohibition) with `recipient neq X` matches a request iff the requesting
// party is NOT X. Missing identity is Unprovable → fail-closed. The recipient is
// resolved from the explicit `odrl:recipient` context OR the requesting party.
// `recipient_status` REPORTS exactly what `evaluate` checks (the honesty point).
// ===========================================================================

const BOB: &str = "https://bob.ex/card#me";
const CAROL: &str = "https://carol.ex/card#me";

/// "everyone EXCEPT bob may read asset-X" — recipient neq bob.
fn recipient_neq_permission() -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/neq> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
                      odrl:rightOperand <{BOB}> ] ] .
"#
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

#[test]
fn recipient_neq_grants_everyone_except_named_party() {
    let p = recipient_neq_permission();
    let base = Request::new(left("read")).on("urn:asset/x");
    // carol is NOT bob → granted (party doubles as recipient).
    assert!(
        evaluate(&p, &base.clone().by(CAROL)).allow,
        "non-excluded party granted"
    );
    // bob is the carved-out recipient → denied.
    assert!(
        !evaluate(&p, &base.clone().by(BOB)).allow,
        "excluded party denied"
    );
}

#[test]
fn recipient_neq_fail_closed_on_missing_identity() {
    let p = recipient_neq_permission();
    // No party AND no explicit recipient context → unprovable → fail-closed DENY.
    let anon = Request::new(left("read")).on("urn:asset/x");
    assert!(
        !evaluate(&p, &anon).allow,
        "no identity → does NOT grant a neq permission"
    );
}

#[test]
fn recipient_neq_explicit_context_overrides_party() {
    // An explicit odrl:recipient context value is the disclosure target even when a
    // (different) party is present — the recipient-of-data need not be the principal.
    let p = recipient_neq_permission();
    let base = Request::new(left("read")).on("urn:asset/x").by(CAROL);
    // explicit recipient = bob → DENY despite party=carol.
    let to_bob = base.clone().with(left("recipient"), Value::Iri(BOB.into()));
    assert!(
        !evaluate(&p, &to_bob).allow,
        "explicit recipient bob is carved out"
    );
    // explicit recipient = carol → granted.
    let to_carol = base.with(left("recipient"), Value::Iri(CAROL.into()));
    assert!(
        evaluate(&p, &to_carol).allow,
        "explicit non-excluded recipient granted"
    );
}

#[test]
fn recipient_neq_prohibition_dual() {
    // "bob is PROHIBITED from reading unless he is NOT bob" — a prohibition with
    // recipient neq bob carves out everyone EXCEPT bob (the dual of the permission).
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/pneq> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
                      odrl:rightOperand <{BOB}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("read")).on("urn:asset/x");
    // carol (not bob) → the prohibition's neq holds → carved out → DENY.
    assert_eq!(
        prohibition_status(&p, &base.clone().by(CAROL)),
        ProhibitionStatus::Applies
    );
    // bob → neq is definitely false → NOT carved out by this prohibition → Withdrawn.
    assert_eq!(
        prohibition_status(&p, &base.clone().by(BOB)),
        ProhibitionStatus::Withdrawn
    );
    // no identity → unprovable → Ambiguous (deny is kept, fail-closed).
    assert_eq!(prohibition_status(&p, &base), ProhibitionStatus::Ambiguous);
}

#[test]
fn recipient_status_reports_what_evaluate_checks() {
    let p = recipient_neq_permission();
    let rule = &p.permissions[0];
    let base = Request::new(left("read")).on("urn:asset/x");
    assert_eq!(
        recipient_status(rule, &base.clone().by(CAROL)),
        RecipientMatch::Satisfied
    );
    assert_eq!(
        recipient_status(rule, &base.clone().by(BOB)),
        RecipientMatch::DefinitelyUnsatisfied
    );
    assert_eq!(recipient_status(rule, &base), RecipientMatch::Unprovable);
    // A rule with NO recipient constraint → NotConstrained.
    let bare = parse_policy_str(
        &format!(
            r#"@prefix odrl: <{ODRL}> . <urn:pol/b> a odrl:Set ; odrl:permission [
               odrl:action odrl:read ; odrl:target <urn:asset/x> ] ."#
        ),
        "turtle",
    )
    .unwrap();
    assert_eq!(
        recipient_status(&bare.permissions[0], &base.by(CAROL)),
        RecipientMatch::NotConstrained
    );
}

// ===========================================================================
// [OPUS-4.8] sq-5037 follow-up — COMBINED recipient `eq A AND neq B` in ONE rule.
// The per-head exception (`agents=[A]`, `except=[B]`) is structurally emitted by the
// bridge but was untested at the evaluator level. Both constraints are ANDed: the
// recipient must BE A and must NOT BE B. (A and B distinct, so only A is ever granted;
// the neq is a redundant-but-honoured second guard exercising the conjunction path.)
// ===========================================================================
#[test]
fn recipient_eq_a_and_neq_b_combined() {
    // "carol (and only carol), but never bob, may read asset-X" — two recipient
    // constraints on ONE permission (eq carol AND neq bob), logically ANDed.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/comb> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
                      odrl:rightOperand <{CAROL}> ] ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
                      odrl:rightOperand <{BOB}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let rule = &p.permissions[0];
    let base = Request::new(left("read")).on("urn:asset/x");

    // carol: eq carol ✓ AND neq bob ✓ → BOTH hold → granted.
    assert!(
        evaluate(&p, &base.clone().by(CAROL)).allow,
        "carol satisfies eq carol AND neq bob"
    );
    assert_eq!(
        recipient_status(rule, &base.clone().by(CAROL)),
        RecipientMatch::Satisfied
    );

    // bob: eq carol ✗ (and neq bob ✗ too) → DENY; the conjunction reports a definite no.
    assert!(
        !evaluate(&p, &base.clone().by(BOB)).allow,
        "bob fails eq carol"
    );
    assert_eq!(
        recipient_status(rule, &base.clone().by(BOB)),
        RecipientMatch::DefinitelyUnsatisfied
    );

    // dave: eq carol ✗ → DENY (and neq bob ✓ — but the AND still fails).
    let dave = "https://dave.ex/card#me";
    assert!(
        !evaluate(&p, &base.clone().by(dave)).allow,
        "dave is not carol"
    );
    assert_eq!(
        recipient_status(rule, &base.clone().by(dave)),
        RecipientMatch::DefinitelyUnsatisfied
    );

    // No identity at all → Unprovable → fail-closed (neither constraint provable).
    assert!(!evaluate(&p, &base).allow, "missing identity → fail-closed");
    assert_eq!(recipient_status(rule, &base), RecipientMatch::Unprovable);
}

#[test]
fn recipient_eq_a_and_neq_b_prohibition_dual() {
    // The prohibition dual: a carve-out gated on `recipient eq carol AND neq bob`. Only
    // carol satisfies BOTH, so only carol's request is carved out; everyone else (incl.
    // bob, who fails eq carol) is NOT carved out → Withdrawn.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/combp> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
                      odrl:rightOperand <{CAROL}> ] ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
                      odrl:rightOperand <{BOB}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("read")).on("urn:asset/x");
    // carol: both hold → carved out → Applies.
    assert_eq!(
        prohibition_status(&p, &base.clone().by(CAROL)),
        ProhibitionStatus::Applies
    );
    // bob: eq carol definitely false → not carved out → Withdrawn.
    assert_eq!(
        prohibition_status(&p, &base.clone().by(BOB)),
        ProhibitionStatus::Withdrawn
    );
    // no identity → unprovable → Ambiguous (deny kept, fail-closed).
    assert_eq!(prohibition_status(&p, &base), ProhibitionStatus::Ambiguous);
}

// ===========================================================================
// [OPUS-4.8] sq-idnv — odrl:dateTime time-window enforcement + the `datetime_status`
// audit helper (the temporal dual of purpose_status / recipient_status). A time-gated
// rule grants ONLY when the request supplies an instant inside the window; a missing
// time is Unprovable → fail-closed; a lapsed/early instant is a definite no.
// ===========================================================================

/// "may read asset-X ONLY until 2026-12-31T23:59:59Z" — an upper time bound.
fn windowed_read_permission() -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <{XSD_DT_NS}> .
<urn:pol/win> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
                      odrl:rightOperand "2026-12-31T23:59:59Z"^^xsd:dateTime ] ] .
"#
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

#[test]
fn datetime_status_reports_what_evaluate_checks() {
    let p = windowed_read_permission();
    let rule = &p.permissions[0];
    let base = Request::new(left("read")).on("urn:asset/x");

    // Inside the window → Satisfied AND evaluate grants. The Request::at sugar carries
    // the instant as odrl:dateTime evidence (auditable via request_time).
    let inside = base.clone().at("2026-06-16T09:00:00Z");
    assert_eq!(datetime_status(rule, &inside), DateTimeMatch::Satisfied);
    assert!(evaluate(&p, &inside).allow, "inside window grants");
    assert_eq!(
        inside.request_time().map(Value::as_str),
        Some("2026-06-16T09:00:00Z")
    );

    // After the window → DefinitelyUnsatisfied AND evaluate denies.
    let lapsed = base.clone().at("2027-03-01T00:00:00Z");
    assert_eq!(
        datetime_status(rule, &lapsed),
        DateTimeMatch::DefinitelyUnsatisfied
    );
    assert!(!evaluate(&p, &lapsed).allow, "after window denies");

    // No time supplied → Unprovable → fail-closed (NOT "any time allowed").
    assert_eq!(datetime_status(rule, &base), DateTimeMatch::Unprovable);
    assert!(!evaluate(&p, &base).allow, "missing time fails closed");
    assert_eq!(base.request_time(), None, "no time evidence supplied");
}

#[test]
fn datetime_two_sided_window_anded() {
    // A two-sided window: gteq lower AND lteq upper — both must hold (conjunction).
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <{XSD_DT_NS}> .
<urn:pol/win2> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gteq ;
                      odrl:rightOperand "2026-01-01T00:00:00Z"^^xsd:dateTime ] ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
                      odrl:rightOperand "2026-12-31T23:59:59Z"^^xsd:dateTime ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let rule = &p.permissions[0];
    let base = Request::new(left("read")).on("urn:asset/x");

    // Inside both bounds → Satisfied.
    assert_eq!(
        datetime_status(rule, &base.clone().at("2026-06-16T00:00:00Z")),
        DateTimeMatch::Satisfied
    );
    // Before the lower bound → one constraint definitely false → DefinitelyUnsatisfied.
    assert_eq!(
        datetime_status(rule, &base.clone().at("2025-12-31T23:59:59Z")),
        DateTimeMatch::DefinitelyUnsatisfied
    );
    // After the upper bound → DefinitelyUnsatisfied.
    assert_eq!(
        datetime_status(rule, &base.clone().at("2027-01-01T00:00:00Z")),
        DateTimeMatch::DefinitelyUnsatisfied
    );
    // Missing time → Unprovable.
    assert_eq!(datetime_status(rule, &base), DateTimeMatch::Unprovable);
}

#[test]
fn datetime_prohibition_dual() {
    // The dual: a prohibition gated on a time window carves the request out ONLY while
    // the window holds; a lapsed window is definitely Withdrawn; missing time is
    // Ambiguous (the deny is NOT withdrawn — fail-closed). "alice prohibited until
    // 2026-01-01" (a `lt` lower edge — the prohibition lapses once we reach the bound).
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <{XSD_DT_NS}> .
<urn:pol/pwin> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ; odrl:assignee <https://alice.ex/me> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
                      odrl:rightOperand "2026-01-01T00:00:00Z"^^xsd:dateTime ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let prohib = &p.prohibitions[0];
    let base = Request::new(left("read"))
        .on("urn:asset/x")
        .by("https://alice.ex/me");

    // Now < bound → the window holds → carve-out Applies; datetime_status Satisfied.
    let inside = base.clone().at("2025-06-01T00:00:00Z");
    assert_eq!(datetime_status(prohib, &inside), DateTimeMatch::Satisfied);
    assert_eq!(prohibition_status(&p, &inside), ProhibitionStatus::Applies);

    // Now >= bound → the window lapsed → definitely no → Withdrawn.
    let lapsed = base.clone().at("2026-06-01T00:00:00Z");
    assert_eq!(
        datetime_status(prohib, &lapsed),
        DateTimeMatch::DefinitelyUnsatisfied
    );
    assert_eq!(
        prohibition_status(&p, &lapsed),
        ProhibitionStatus::Withdrawn
    );

    // No time → unprovable → Ambiguous (deny kept).
    assert_eq!(datetime_status(prohib, &base), DateTimeMatch::Unprovable);
    assert_eq!(prohibition_status(&p, &base), ProhibitionStatus::Ambiguous);
}

#[test]
fn datetime_not_constrained_when_absent() {
    // A rule with no dateTime constraint → NotConstrained (time places no bound).
    let ttl = format!(
        r#"@prefix odrl: <{ODRL}> . <urn:pol/nt> a odrl:Set ; odrl:permission [
           odrl:action odrl:read ; odrl:target <urn:asset/x> ] ."#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .at("2026-06-16T00:00:00Z");
    assert_eq!(
        datetime_status(&p.permissions[0], &req),
        DateTimeMatch::NotConstrained
    );
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-qj2q — mixed-offset dateTime normalization. The instant
// 2026-06-16T12:00:00Z equals 2026-06-16T13:00:00+01:00 and is one hour LATER
// than 2026-06-16T13:00:00+02:00 (=11:00Z). A lexical comparison gets all of
// these wrong; the evaluator must normalize to the instant before comparing.
// ---------------------------------------------------------------------------

/// A `dateTime lteq T` policy whose upper bound `T` is supplied with an offset.
fn lteq_window(bound: &str) -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/tz> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
                      odrl:rightOperand "{bound}"^^<{XSD_DT}> ] ] .
"#
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

#[test]
fn mixed_offset_request_under_z_bound_allows() {
    // Bound 2026-06-16T12:00:00Z. Request 2026-06-16T13:00:00+02:00 = 11:00Z,
    // one hour BEFORE the bound → inside the lteq window → ALLOW. A lexical
    // compare would read "13:00:00+02:00" > "12:00:00Z" → wrongly DENY.
    let p = lteq_window("2026-06-16T12:00:00Z");
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .at("2026-06-16T13:00:00+02:00");
    assert!(evaluate(&p, &req).allow, "11:00Z must be <= 12:00Z");
    assert_eq!(
        datetime_status(&p.permissions[0], &req),
        DateTimeMatch::Satisfied
    );
}

#[test]
fn mixed_offset_request_over_z_bound_denies() {
    // Bound 2026-06-16T12:00:00Z. Request 2026-06-16T13:00:00-02:00 = 15:00Z,
    // AFTER the bound → outside the lteq window → DENY. A lexical compare would
    // read "13:00:00-02:00" > "12:00:00Z" too, but for the wrong reason; flip
    // the sign to be sure the offset is actually applied.
    let p = lteq_window("2026-06-16T12:00:00Z");
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .at("2026-06-16T13:00:00-02:00");
    assert!(!evaluate(&p, &req).allow, "15:00Z must be > 12:00Z");
    assert_eq!(
        datetime_status(&p.permissions[0], &req),
        DateTimeMatch::DefinitelyUnsatisfied
    );
}

#[test]
fn offset_bound_normalized_against_z_request() {
    // The OFFSET lives on the policy bound this time. Bound 2026-06-16T13:00:00+02:00
    // = 11:00Z. Request 2026-06-16T12:00:00Z = 12:00Z > bound → DENY (lexical
    // would read "12:00:00Z" < "13:00:00+02:00" → wrongly ALLOW).
    let p = lteq_window("2026-06-16T13:00:00+02:00");
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .at("2026-06-16T12:00:00Z");
    assert!(!evaluate(&p, &req).allow, "12:00Z must be > 11:00Z bound");
}

#[test]
fn equal_instants_distinct_offsets_compare_equal() {
    // eq operator: 2026-06-16T12:00:00Z == 2026-06-16T14:00:00+02:00 (both 12:00Z).
    // A lexical eq sees different strings → wrongly DENY.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/eq> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:eq ;
                      odrl:rightOperand "2026-06-16T12:00:00Z"^^<{XSD_DT}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .at("2026-06-16T14:00:00+02:00");
    assert!(evaluate(&p, &req).allow, "same instant must satisfy eq");
    // And neq of the same instant must be false → a neq-gated rule denies.
    let ttl_neq = ttl.replace("odrl:eq", "odrl:neq");
    let p_neq = parse_policy_str(&ttl_neq, "turtle").unwrap();
    assert!(
        !evaluate(&p_neq, &req).allow,
        "same instant must fail neq (still equal)"
    );
}

#[test]
fn fractional_seconds_normalized() {
    // Fractional seconds + offset: 2026-06-16T12:00:00.500+01:00 = 11:00:00.5Z,
    // strictly before the 12:00:00Z bound under lt.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/fr> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
                      odrl:rightOperand "2026-06-16T12:00:00Z"^^<{XSD_DT}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .at("2026-06-16T12:00:00.500+01:00");
    assert!(evaluate(&p, &req).allow, "11:00:00.5Z must be < 12:00:00Z");
}

#[test]
fn date_only_normalized_against_offset_datetime() {
    // xsd:date (no time) is treated as the start of day. 2026-06-16 (=00:00Z)
    // vs a request at 2026-06-15T23:00:00-02:00 (=2026-06-16T01:00:00Z) under
    // gteq: request instant 01:00Z >= bound 00:00Z → ALLOW.
    let xsd_date = "http://www.w3.org/2001/XMLSchema#date";
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/d> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gteq ;
                      odrl:rightOperand "2026-06-16"^^<{xsd_date}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .at("2026-06-15T23:00:00-02:00");
    assert!(
        evaluate(&p, &req).allow,
        "01:00Z on the 16th must be >= 00:00Z"
    );
}

#[test]
fn unparseable_datetime_falls_back_to_lexical_and_denies_order() {
    // Garbage that is not a valid instant must NOT panic and must NOT silently
    // grant: an order comparison against an unparseable operand is fail-closed.
    let p = lteq_window("2026-06-16T12:00:00Z");
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .with(left("dateTime"), dt("not-a-date"));
    assert!(
        !evaluate(&p, &req).allow,
        "unparseable instant must not grant"
    );
}

// ===========================================================================
// [OPUS-4.8] sq-z3ve — DPV/purpose-taxonomy hierarchy/subsumption matching.
// A purpose constraint `eq B` (or `isA B` / a member of an `isPartOf` set) is
// satisfied by a request whose stated purpose is B *or transitively narrower
// than B* (P ⊑ B), where the ⊑ edges are an EXPLICIT taxonomy the request
// supplies (DPV `skos:broader`/`rdfs:subClassOf` closure). SOUNDNESS: with no
// supplied taxonomy, matching is byte-for-byte identical to the exact-IRI base
// case; subsumption only ever comes from caller-supplied edges (never invented
// from IRI string structure), so access is never widened on an unproven
// relation. `neq B` is correspondingly *unsatisfied* for a sub-purpose of B
// (a sub-purpose IS a B-purpose, so it must not slip past a `neq B` carve-out).
// ===========================================================================

const CLINICAL: &str = "urn:purpose/research/clinical";

#[test]
fn purpose_subsumption_narrower_grants_under_broader_permission() {
    // Permission gated on `eq research`; request states the NARROWER `clinical`
    // purpose + the taxonomy edge clinical ⊑ research → grant.
    let p = purpose_permission(); // gated on <urn:purpose/research>
    let req = Request::new(left("use"))
        .on("urn:asset/x")
        .by("https://alice.ex/me")
        .for_purpose(Value::Iri(CLINICAL.into()))
        .with_purpose_subsumption(CLINICAL, RESEARCH);
    assert!(
        evaluate(&p, &req).allow,
        "clinical ⊑ research → a research permission covers it"
    );
    assert_eq!(
        purpose_status(&p.permissions[0], &req),
        PurposeMatch::Satisfied
    );
}

#[test]
fn purpose_subsumption_is_transitive() {
    // grandchild ⊑ child ⊑ research → a research permission covers the grandchild.
    let grandchild = "urn:purpose/research/clinical/trial";
    let p = purpose_permission(); // gated on <urn:purpose/research>
    let req = Request::new(left("use"))
        .on("urn:asset/x")
        .by("https://alice.ex/me")
        .for_purpose(Value::Iri(grandchild.into()))
        .with_purpose_subsumption(grandchild, CLINICAL)
        .with_purpose_subsumption(CLINICAL, RESEARCH);
    assert!(
        evaluate(&p, &req).allow,
        "subsumption is transitive: grandchild ⊑ research"
    );
}

#[test]
fn purpose_subsumption_broader_does_not_grant_under_narrower_permission() {
    // SOUNDNESS / direction: a permission gated on `eq clinical` is NOT satisfied
    // by a request stating the BROADER `research` purpose — broader ⋢ narrower.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/pc> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
                      odrl:rightOperand <{CLINICAL}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("use"))
        .on("urn:asset/x")
        .for_purpose(Value::Iri(RESEARCH.into()))
        .with_purpose_subsumption(CLINICAL, RESEARCH);
    assert!(
        !evaluate(&p, &req).allow,
        "broader research ⋢ narrower clinical: must not grant"
    );
    assert_eq!(
        purpose_status(&p.permissions[0], &req),
        PurposeMatch::DefinitelyUnsatisfied
    );
}

#[test]
fn purpose_subsumption_unrelated_does_not_grant() {
    // A taxonomy that does NOT relate the stated purpose to the constraint's
    // purpose leaves matching exactly as the exact-IRI base case (no grant).
    let p = purpose_permission(); // gated on <urn:purpose/research>
    let req = Request::new(left("use"))
        .on("urn:asset/x")
        .by("https://alice.ex/me")
        .for_purpose(Value::Iri(MARKETING.into()))
        .with_purpose_subsumption(CLINICAL, RESEARCH); // irrelevant edge
    assert!(
        !evaluate(&p, &req).allow,
        "marketing is not ⊑ research under the supplied taxonomy"
    );
}

#[test]
fn purpose_no_taxonomy_is_exact_match_only() {
    // SOUNDNESS regression: WITHOUT a supplied taxonomy, a sub-purpose IRI is
    // still NOT subsumed — identical to the pre-existing exact-match behaviour.
    let p = purpose_permission(); // gated on <urn:purpose/research>
    let req = Request::new(left("use"))
        .on("urn:asset/x")
        .by("https://alice.ex/me")
        .for_purpose(Value::Iri(CLINICAL.into())); // no .with_purpose_subsumption
    assert!(
        !evaluate(&p, &req).allow,
        "no taxonomy ⇒ exact match only (unchanged base case)"
    );
}

#[test]
fn purpose_subsumption_via_is_part_of_set() {
    // `isPartOf` over a set {research|marketing}: a request stating the narrower
    // `clinical` (clinical ⊑ research) is a member-by-subsumption → grant.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/ps2> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand "{RESEARCH}|{MARKETING}" ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("use"))
        .on("urn:asset/x")
        .for_purpose(Value::Iri(CLINICAL.into()))
        .with_purpose_subsumption(CLINICAL, RESEARCH);
    assert!(
        evaluate(&p, &req).allow,
        "clinical ⊑ research ∈ set → member by subsumption"
    );
}

#[test]
fn purpose_neq_carves_out_sub_purposes_too() {
    // SOUNDNESS: a `neq research` carve-out must ALSO exclude a sub-purpose of
    // research (clinical research IS a research purpose). Stating `clinical`
    // with the edge clinical ⊑ research must NOT grant under `neq research`.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/pn2> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:neq ;
                      odrl:rightOperand <{RESEARCH}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("use")).on("urn:asset/x");
    // A sub-purpose of the excluded research purpose → still excluded.
    let sub = base
        .clone()
        .for_purpose(Value::Iri(CLINICAL.into()))
        .with_purpose_subsumption(CLINICAL, RESEARCH);
    assert!(
        !evaluate(&p, &sub).allow,
        "neq research must also exclude clinical ⊑ research"
    );
    // An unrelated purpose → not excluded → grant.
    let other = base
        .clone()
        .for_purpose(Value::Iri(MARKETING.into()))
        .with_purpose_subsumption(CLINICAL, RESEARCH);
    assert!(evaluate(&p, &other).allow, "marketing ⋢ research → granted");
}

#[test]
fn purpose_subsumption_prohibition_carves_out_sub_purpose() {
    // A prohibition gated on `eq research` carves out a request for the narrower
    // `clinical` purpose too (deny-overrides), since clinical ⊑ research.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/pp> a odrl:Set ;
  odrl:permission [ odrl:action odrl:use ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:use ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
                      odrl:rightOperand <{RESEARCH}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("use"))
        .on("urn:asset/x")
        .for_purpose(Value::Iri(CLINICAL.into()))
        .with_purpose_subsumption(CLINICAL, RESEARCH);
    assert!(
        !evaluate(&p, &req).allow,
        "prohibition on research carves out clinical ⊑ research"
    );
}

#[test]
fn purpose_subsumption_bulk_taxonomy_builder() {
    // The bulk `with_purpose_taxonomy(edges)` builder accepts many (narrower,
    // broader) edges at once and computes the transitive closure.
    let grandchild = "urn:purpose/research/clinical/trial";
    let p = purpose_permission(); // gated on <urn:purpose/research>
    let req = Request::new(left("use"))
        .on("urn:asset/x")
        .by("https://alice.ex/me")
        .for_purpose(Value::Iri(grandchild.into()))
        .with_purpose_taxonomy([(grandchild, CLINICAL), (CLINICAL, RESEARCH)]);
    assert!(
        evaluate(&p, &req).allow,
        "bulk taxonomy builder computes the transitive closure"
    );
}

// ---------------------------------------------------------------------------
// Additional correctness coverage. [OPUS-4.8] sq-bif.
//
// These pin behaviours the suite above did not assert: the **conjunction of
// multiple duties** (every duty must be discharged, not just one), the duty
// **default action** (a duty with no `odrl:action` falls back to the `use`
// umbrella), the `isPartOf` set **separators** (`,` as well as `|`/space), the
// **typed-value boundary** for `isPartOf` (a `Value::Num` carries no string so it
// never matches a set, while the same value as a `Value::Str` does), **cross-type**
// value equality (an `Iri` actual matches a `Str` right-operand with the same
// canonical string — the parser's intentional reading), `odrl:rightOperandReference`
// parsing (an IRI right-operand named via the reference predicate), and the
// **malformed-typed-literal fallback** (an `xsd:integer` whose lexical form is not a
// number is carried as a `Str`, not silently treated as 0 or dropped).
// ---------------------------------------------------------------------------

// A permission carrying TWO duties grants ONLY when BOTH are discharged — the
// duties are conjoined (ANDed). Discharging just one is a DENY: the regression this
// catches is an evaluator that grants on the first (or any) duty rather than all.
#[test]
fn multiple_duties_all_must_be_discharged() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/d2> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:duty [ odrl:action odrl:anonymize ] ;
    odrl:duty [ odrl:action odrl:compensate ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    assert_eq!(p.permissions[0].duties.len(), 2, "both duties parsed");
    let base = Request::new(left("read")).on("urn:asset/x");
    // No duty discharged → DENY.
    assert!(!evaluate(&p, &base).allow);
    // Only ONE of the two discharged → still DENY (the other is undischarged).
    assert!(
        !evaluate(&p, &base.clone().discharge(left("anonymize"))).allow,
        "one of two duties is not enough"
    );
    assert!(
        !evaluate(&p, &base.clone().discharge(left("compensate"))).allow,
        "the other single duty is not enough either"
    );
    // BOTH discharged → ALLOW.
    let both = base
        .discharge(left("anonymize"))
        .discharge(left("compensate"));
    assert!(evaluate(&p, &both).allow, "both duties discharged → grant");
}

// A duty with no explicit `odrl:action` defaults to the `odrl:use` umbrella action
// (the same default a rule's action takes when omitted), so it is discharged by
// `use` — not by some other action and not vacuously.
#[test]
fn duty_with_no_action_defaults_to_use() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/du> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:duty [ odrl:assignee <https://x.ex/me> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    assert_eq!(p.permissions[0].duties.len(), 1);
    assert_eq!(
        p.permissions[0].duties[0].action.0,
        left("use"),
        "an action-less duty defaults to the use umbrella"
    );
    let base = Request::new(left("read")).on("urn:asset/x");
    // Undischarged → DENY; discharging the *use* action → ALLOW.
    assert!(!evaluate(&p, &base).allow);
    assert!(
        evaluate(&p, &base.clone().discharge(left("use"))).allow,
        "the default-use duty is discharged by use"
    );
    // Discharging an UNRELATED action does NOT discharge the use duty.
    assert!(
        !evaluate(&p, &base.discharge(left("anonymize"))).allow,
        "an unrelated discharge does not satisfy the use duty"
    );
}

// `odrl:isPartOf` set membership accepts a comma-separated right operand (not only
// `|`/space-separated), so a recipient listed in `a,b,c` is a member.
#[test]
fn is_part_of_accepts_comma_separated_set() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/cs> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand "a,b,c" ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("read")).on("urn:asset/x");
    // `b` is a member of `a,b,c` → ALLOW.
    let member = base.clone().with(left("recipient"), Value::Str("b".into()));
    assert!(evaluate(&p, &member).allow, "b ∈ {{a,b,c}}");
    // `z` is not a member → DENY.
    let nonmember = base.with(left("recipient"), Value::Str("z".into()));
    assert!(!evaluate(&p, &nonmember).allow, "z ∉ {{a,b,c}}");
}

// The typed-value boundary for `isPartOf`: a `Value::Num` carries no string form, so
// it can never be a member of a string set, whereas the SAME value supplied as a
// `Value::Str` matches. This pins that set membership is a string operation (a
// regression that string-ified numbers would silently widen the set match).
#[test]
fn is_part_of_numeric_actual_never_matches_string_set() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/ns> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:count ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand "1 2 3" ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("read")).on("urn:asset/x");
    // A numeric 2 does NOT match the string set (Num has no string form).
    assert!(
        !evaluate(&p, &base.clone().with(left("count"), Value::Num(2.0))).allow,
        "a Value::Num is not a member of a string set"
    );
    // The same value as a string DOES match the set.
    assert!(
        evaluate(&p, &base.with(left("count"), Value::Str("2".into()))).allow,
        "the string '2' is a member of the set"
    );
}

// Cross-type value equality: an `eq` constraint whose right operand parses as a
// plain-literal `Str` is satisfied by a request value supplied as an `Iri` with the
// same canonical string (and vice-versa). The evaluator compares by canonical string
// across the Iri/Str type split — pinning the parser's intentional reading so a later
// "compare types strictly" change cannot silently start denying.
#[test]
fn eq_matches_across_iri_and_str_value_types() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/ct> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
                      odrl:rightOperand "urn:purpose/r" ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    // The right operand parsed to a Str; an Iri actual with the same string matches.
    assert!(
        matches!(&p.permissions[0].constraints[0].right, Value::Str(s) if s == "urn:purpose/r"),
        "right operand is a plain-literal Str"
    );
    let base = Request::new(left("read")).on("urn:asset/x");
    let iri_actual = base.clone().for_purpose(Value::Iri("urn:purpose/r".into()));
    assert!(
        evaluate(&p, &iri_actual).allow,
        "an Iri actual equals a Str right-operand with the same canonical string"
    );
    // A different string still denies (the canonical-string compare is exact).
    let other = base.for_purpose(Value::Iri("urn:purpose/other".into()));
    assert!(!evaluate(&p, &other).allow);
}

// `odrl:rightOperandReference` is an accepted spelling of the right operand (an IRI
// reference), parsed to the same `Value::Iri` and matched identically to a plain
// `odrl:rightOperand`. A regression that ignored the reference predicate would parse
// a constraint with a missing right operand → the unsatisfiable guard → wrongly DENY.
#[test]
fn right_operand_reference_is_parsed_and_matched() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/ror> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:spatial ; odrl:operator odrl:eq ;
                      odrl:rightOperandReference <urn:country/EU> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    assert_eq!(
        p.permissions[0].constraints.len(),
        1,
        "the reference-form constraint is a well-formed atomic constraint"
    );
    assert_eq!(
        p.permissions[0].constraints[0].right,
        Value::Iri("urn:country/EU".into()),
        "rightOperandReference parses to the same Value::Iri"
    );
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .with(left("spatial"), Value::Iri("urn:country/EU".into()));
    assert!(
        evaluate(&p, &req).allow,
        "a rightOperandReference bound matches like rightOperand"
    );
}

// A typed numeric literal whose lexical form is NOT a number (`"x"^^xsd:integer`)
// falls back to a `Value::Str` rather than being dropped or coerced to 0. The rule
// then still parses (no panic, no silent unsatisfiable guard from a missing operand)
// and an `eq` against the same string succeeds — pinning the fail-soft typing.
#[test]
fn malformed_numeric_literal_falls_back_to_string() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <{XSD_DT_NS}> .
<urn:pol/mn> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:count ; odrl:operator odrl:eq ;
                      odrl:rightOperand "notanumber"^^xsd:integer ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    assert_eq!(
        p.permissions[0].constraints[0].right,
        Value::Str("notanumber".into()),
        "an unparseable xsd:integer is carried as a Str, not coerced/dropped"
    );
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .with(left("count"), Value::Str("notanumber".into()));
    assert!(
        evaluate(&p, &req).allow,
        "the string-fallback bound still compares by string equality"
    );
}
