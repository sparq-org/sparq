//! Multi-valued `odrl:rightOperand` parsing — RDF lists (`odrl:rightOperand
//! ( <a> <b> )`) and multiple objects (`odrl:rightOperand <a>, <b>`) folded into
//! the `|`-separated set encoding the set-relation operators
//! (`isPartOf`/`isAnyOf`/`isNoneOf`) consume. End-to-end through the REAL
//! `parse_policy_str` + `evaluate` path (not a mock). [FABLE-5] sq-ueydm.
//!
//! Load-bearing invariants pinned here:
//! * a single-valued right operand (including a ONE-element list) keeps the
//!   TYPED value path (numeric/dateTime magnitude comparison — no behavior
//!   change to the pre-fold parse);
//! * a multi-value under a NON-set operator (`eq`, …) is ambiguous and fails
//!   CLOSED (the unsatisfiable guard), and the folded `a|b` encoding never
//!   leaks into equality comparison;
//! * a member that carries a set-encoding separator (`|`/whitespace/`,`) cannot
//!   be encoded faithfully — the whole constraint fails closed rather than
//!   splitting into unintended members (a fail-OPEN hazard);
//! * a MALFORMED collection (broken/dangling tail, cycle, forked cell, or a
//!   member-less head asserting `rdf:rest` with no `rdf:first`) REFUSES the whole
//!   parse on BOTH rule kinds rather than contributing its valid prefix — or, for
//!   the member-less head, being read as an ordinary unmatchable value (sq-srjuc —
//!   both drop authored members and widen decisions).

use sparq_policy::{evaluate, parse_policy_str, Operator, Request, Value};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

fn action(local: &str) -> String {
    format!("{}{}", ODRL, local)
}

/// A permission gated on `purpose <op> <right>` (TTL helper — same shape as
/// `odrl_set_operators.rs`).
fn purpose_policy(op: &str, right_ttl: &str) -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:{} ;
                      odrl:rightOperand {} ] ] .
"#,
        op, right_ttl
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

fn read_x() -> Request {
    Request::new(action("read")).on("urn:asset/x")
}

fn purpose(iri: &str) -> Request {
    read_x().for_purpose(Value::Iri(iri.to_owned()))
}

// ===========================================================================
// Structural: the parsed constraint carries the folded set encoding.
// ===========================================================================

/// An RDF-list right operand folds to the `|`-joined set-encoding string, in
/// list order — a DIRECT parse-level assertion on the encoding (not only on
/// evaluation behavior).
#[test]
fn list_right_operand_folds_to_set_encoding() {
    let p = purpose_policy("isAnyOf", "( <urn:purpose/research> <urn:purpose/education> )");
    let c = &p.permissions[0].constraints[0];
    assert_eq!(c.operator, Operator::IsAnyOf);
    assert_eq!(
        c.right,
        Value::Str("urn:purpose/research|urn:purpose/education".to_owned()),
        "rdf:first/rdf:rest chain must fold into the |-separated set encoding"
    );
}

/// Multiple `rightOperand` objects fold into ONE constraint carrying the full
/// member set (object order in RDF is unspecified, so assert set equality).
#[test]
fn multi_object_right_operand_folds_to_set_encoding() {
    let p = purpose_policy("isAnyOf", "<urn:purpose/research>, <urn:purpose/education>");
    assert_eq!(
        p.permissions[0].constraints.len(),
        1,
        "several objects of one rightOperand are ONE constraint, not several"
    );
    let c = &p.permissions[0].constraints[0];
    let Value::Str(s) = &c.right else {
        panic!("multi-object rightOperand must fold to Value::Str, got {:?}", c.right);
    };
    let mut members: Vec<&str> = s.split('|').collect();
    members.sort_unstable();
    assert_eq!(members, ["urn:purpose/education", "urn:purpose/research"]);
}

// ===========================================================================
// Evaluation: list / multi-object sets behave as faithful set membership.
// ===========================================================================

/// `isAnyOf ( <a> <b> )`: every listed member grants, a non-member denies.
#[test]
fn is_any_of_list_member_grants_non_member_denied() {
    let p = purpose_policy("isAnyOf", "( <urn:purpose/research> <urn:purpose/education> )");
    assert!(evaluate(&p, &purpose("urn:purpose/research")).allow);
    assert!(evaluate(&p, &purpose("urn:purpose/education")).allow);
    assert!(!evaluate(&p, &purpose("urn:purpose/marketing")).allow);
}

/// `isPartOf` consumes the same folded encoding (set membership).
#[test]
fn is_part_of_list_member_grants_non_member_denied() {
    let p = purpose_policy("isPartOf", "( <urn:purpose/research> <urn:purpose/education> )");
    assert!(evaluate(&p, &purpose("urn:purpose/education")).allow);
    assert!(!evaluate(&p, &purpose("urn:purpose/marketing")).allow);
}

/// Multi-object `isAnyOf <a>, <b>`: both members grant, a non-member denies —
/// previously first-binding-wins silently dropped the second member.
#[test]
fn is_any_of_multi_object_every_member_grants() {
    let p = purpose_policy("isAnyOf", "<urn:purpose/research>, <urn:purpose/education>");
    assert!(evaluate(&p, &purpose("urn:purpose/research")).allow);
    assert!(
        evaluate(&p, &purpose("urn:purpose/education")).allow,
        "the second rightOperand object must not be dropped"
    );
    assert!(!evaluate(&p, &purpose("urn:purpose/marketing")).allow);
}

/// `isNoneOf ( <a> <b> )` on a permission: a non-member grants, each listed
/// member is a definite mismatch (deny).
#[test]
fn is_none_of_list_excludes_every_member() {
    let p = purpose_policy("isNoneOf", "( <urn:purpose/a> <urn:purpose/b> )");
    assert!(evaluate(&p, &purpose("urn:purpose/c")).allow);
    assert!(!evaluate(&p, &purpose("urn:purpose/a")).allow);
    assert!(
        !evaluate(&p, &purpose("urn:purpose/b")).allow,
        "the second list member must also be excluded"
    );
}

/// `isNoneOf` list on a PROHIBITION: the prohibition fires (deny) exactly when
/// the stated purpose is outside the set — the folded set drives the negative
/// operator faithfully in the deny direction too.
#[test]
fn prohibition_is_none_of_list_fires_outside_the_set() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isNoneOf ;
                      odrl:rightOperand ( <urn:purpose/a> <urn:purpose/b> ) ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // Purpose outside the set → prohibition satisfied → DENY overrides the permission.
    assert!(!evaluate(&p, &purpose("urn:purpose/c")).allow);
    // Purpose in the set → prohibition constraint unsatisfied → the permission grants.
    assert!(evaluate(&p, &purpose("urn:purpose/b")).allow);
}

/// A list right operand inside a compound `odrl:LogicalConstraint` operand is
/// folded through the same path (the logical-constraint atom table). The
/// combinator's operands stay several objects (`odrl:or _:c1` — the suite's
/// form); the LIST is the operand's `rightOperand`.
#[test]
fn logical_constraint_operand_folds_list_right_operand() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:or _:c1 ] ] .
_:c1 odrl:leftOperand odrl:purpose ; odrl:operator odrl:isAnyOf ;
     odrl:rightOperand ( <urn:purpose/research> <urn:purpose/education> ) .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert_eq!(
        p.permissions[0].logical_constraints.len(),
        1,
        "the compound constraint must parse (this test must not pass vacuously)"
    );
    assert!(evaluate(&p, &purpose("urn:purpose/education")).allow);
    assert!(!evaluate(&p, &purpose("urn:purpose/marketing")).allow);
}

// ===========================================================================
// Typed single-value path is unchanged.
// ===========================================================================

/// A ONE-element list stays TYPED: `dateTime lteq ( "…"^^xsd:dateTime )`
/// compares by instant, proving the single-member fold takes the typed
/// `value_of` path (not a lexical set string).
#[test]
fn single_element_list_keeps_typed_comparison() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
                      odrl:rightOperand ( "2026-12-31T00:00:00Z"^^xsd:dateTime ) ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let c = &p.permissions[0].constraints[0];
    assert!(
        matches!(c.right, Value::DateTime(_)),
        "one-element list must keep the typed dateTime value, got {:?}",
        c.right
    );
    assert!(evaluate(&p, &read_x().at("2026-06-01T00:00:00Z")).allow);
    assert!(!evaluate(&p, &read_x().at("2027-06-01T00:00:00Z")).allow);
}

// ===========================================================================
// Fail-closed degradations.
// ===========================================================================

/// A multi-value under a NON-set operator (`eq`) is ambiguous → the constraint
/// is unsatisfiable (fail-closed), and the folded `a|b` encoding must NOT leak
/// into equality: even a request value that IS literally the joined string
/// never grants.
#[test]
fn eq_with_multiple_values_fails_closed() {
    let p = purpose_policy("eq", "<urn:purpose/a>, <urn:purpose/b>");
    assert!(!evaluate(&p, &purpose("urn:purpose/a")).allow);
    assert!(!evaluate(&p, &purpose("urn:purpose/b")).allow);
    let joined = read_x().for_purpose(Value::Str("urn:purpose/a|urn:purpose/b".to_owned()));
    assert!(
        !evaluate(&p, &joined).allow,
        "the fold encoding must not become spuriously equal-comparable"
    );
}

/// A set member carrying a separator character (here: a space) cannot be
/// encoded faithfully — the WHOLE constraint fails closed: neither the clean
/// member, nor the dirty member, nor its would-be split fragments grant.
#[test]
fn separator_carrying_member_fails_the_whole_constraint_closed() {
    let p = purpose_policy("isAnyOf", r#"( "two words" <urn:purpose/b> )"#);
    assert!(
        !evaluate(&p, &purpose("urn:purpose/b")).allow,
        "an unencodable sibling member must fail the whole constraint closed"
    );
    for v in ["two words", "two", "words"] {
        let req = read_x().for_purpose(Value::Str(v.to_owned()));
        assert!(!evaluate(&p, &req).allow, "value {:?} must not grant", v);
    }
}

/// An EMPTY list (`rdf:nil` directly as the object) has no members: ordinary
/// request values never match (fail-closed, same as the pre-fold parse).
#[test]
fn empty_list_right_operand_never_matches_ordinary_values() {
    let p = purpose_policy("isAnyOf", "( )");
    assert!(!evaluate(&p, &purpose("urn:purpose/a")).allow);
}

// ===========================================================================
// sq-srjuc — a MALFORMED collection rightOperand REFUSES the parse ([FABLE-5]).
//
// The fold used to walk the chain prefix-tolerantly, contributing the valid
// PREFIX of a broken-tail/dangling/forked collection (and, for a cycle, the full
// member set of the loop). Dropping an authored member from a SET encoding widens
// decisions on both rule kinds — `isNoneOf` on a permission excludes fewer values
// than authored (the dropped member now grants), and any set-op constraint gating
// a PROHIBITION narrows the carve-out, so deny-overrides is bypassed. Degrading
// the single constraint to the unsatisfiable guard is not a sound fallback either
// (an unsatisfiable constraint DISABLES a prohibition — the same direction), so
// the whole parse is refused, matching `fold_list_operands` (sq-dkuff).
//
// Written with explicit cons-cell triples: Turtle's `( … )` sugar always emits
// well-formed collections, so these shapes cannot be authored any other way.
// ===========================================================================

/// The `isNoneOf`-on-a-permission widening witness: a BROKEN TAIL (`rdf:first`
/// with no `rdf:rest`) truncates `{a, b}` to `{a}`, and `purpose isNoneOf a`
/// would then GRANT purpose `b` — which the policy author excluded. Refused.
#[test]
fn broken_tail_right_operand_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isNoneOf ;
                      odrl:rightOperand _:l1 ] ] .
_:l1 rdf:first <urn:purpose/a> .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection rightOperand"),
        "a broken-tail rightOperand list must refuse the parse, got: {err}"
    );
}

/// A DANGLING tail (`rdf:rest` pointing at a node that is neither a cons cell nor
/// `rdf:nil`) has no well-defined member set — the walk cannot know whether members
/// were lost. Refused rather than truncated to the reachable prefix.
#[test]
fn dangling_tail_right_operand_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isAnyOf ;
                      odrl:rightOperand _:l1 ] ] .
_:l1 rdf:first <urn:purpose/a> ; rdf:rest <urn:not/a-cons-cell> .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection rightOperand"),
        "a dangling-tail rightOperand list must refuse the parse, got: {err}"
    );
}

/// A CYCLIC list never reaches `rdf:nil`, so the collection has no well-defined
/// member set even though the loop happens to enumerate every cell. Refused.
#[test]
fn cyclic_right_operand_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isAnyOf ;
                      odrl:rightOperand _:l1 ] ] .
_:l1 rdf:first <urn:purpose/a> ; rdf:rest _:l2 .
_:l2 rdf:first <urn:purpose/b> ; rdf:rest _:l1 .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection rightOperand"),
        "a cyclic rightOperand list must refuse the parse, got: {err}"
    );
}

/// A FORKED cons cell (two distinct `rdf:rest` values) is ambiguous: honouring one
/// deterministic fork silently drops the other branch's authored members. Refused.
#[test]
fn forked_cell_right_operand_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isAnyOf ;
                      odrl:rightOperand _:l1 ] ] .
_:l1 rdf:first <urn:purpose/a> ; rdf:rest _:l2 , rdf:nil .
_:l2 rdf:first <urn:purpose/b> ; rdf:rest rdf:nil .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection rightOperand"),
        "a forked rightOperand cons cell must refuse the parse, got: {err}"
    );
}

/// The PROHIBITION direction, on the rule kind where degrading to the
/// unsatisfiable guard would itself widen: a truncated `isPartOf` set narrows the
/// carve-out, so the sibling permission grants where the author denied. Refused —
/// on the prohibition too, not only on permissions.
#[test]
fn malformed_right_operand_on_prohibition_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand _:l1 ] ] .
_:l1 rdf:first <urn:purpose/a> ; rdf:rest _:l2 .
_:l2 rdf:first <urn:purpose/b> .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection rightOperand"),
        "a malformed rightOperand list gating a PROHIBITION must refuse the parse, got: {err}"
    );
}

/// A HEAD-position REST-ONLY cell (`rdf:rest` with no `rdf:first`) is invisible to
/// the `rdf:first`-keyed cons-cell table, so before sq-srjuc it bypassed collection
/// validation entirely and folded as one ordinary value — the unmatchable blank node
/// `_:l1`. On a PROHIBITION that is the widening direction: the carve-out's
/// `isPartOf` constraint can never be satisfied, the prohibition never fires, and the
/// sibling permission GRANTS. Refused instead, like every other malformed shape.
#[test]
fn rest_only_head_right_operand_on_prohibition_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand _:l1 ] ] .
_:l1 rdf:rest _:l2 .
_:l2 rdf:first <urn:purpose/a> ; rdf:rest rdf:nil .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection rightOperand"),
        "a member-less rest-only rightOperand head gating a PROHIBITION must refuse the \
         parse (else the prohibition is disabled and the sibling permission grants), got: {err}"
    );
}

/// The permission direction of the same shape, so the refusal is not prohibition-only:
/// a rest-only head under `isNoneOf` would exclude nothing at all.
#[test]
fn rest_only_head_right_operand_on_permission_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isNoneOf ;
                      odrl:rightOperand _:l1 ] ] .
_:l1 rdf:rest rdf:nil .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection rightOperand"),
        "a member-less rest-only rightOperand head must refuse the parse, got: {err}"
    );
}

/// The same refusal reaches the *other* fold entry point: a rightOperand list on an
/// atomic constraint nested inside a compound `odrl:LogicalConstraint` (the
/// logical-constraint atom table), here on a prohibition's carve-out.
#[test]
fn malformed_right_operand_inside_logical_constraint_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or _:c1 ] ] .
_:c1 odrl:leftOperand odrl:purpose ; odrl:operator odrl:isNoneOf ;
     odrl:rightOperand _:l1 .
_:l1 rdf:first <urn:purpose/a> ; rdf:rest _:l1 .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection rightOperand"),
        "a malformed rightOperand inside a compound must refuse the parse, got: {err}"
    );
}

/// Guard against over-refusal: a WELL-FORMED rightOperand collection that happens
/// to be written as explicit cons cells (not Turtle `( … )` sugar) still parses and
/// folds to the complete set encoding.
#[test]
fn explicit_well_formed_cons_cells_still_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isAnyOf ;
                      odrl:rightOperand _:l1 ] ] .
_:l1 rdf:first <urn:purpose/a> ; rdf:rest _:l2 .
_:l2 rdf:first <urn:purpose/b> ; rdf:rest rdf:nil .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert_eq!(
        p.permissions[0].constraints[0].right,
        Value::Str("urn:purpose/a|urn:purpose/b".to_owned()),
        "a well-formed explicit cons-cell list must still fold to the full set"
    );
    assert!(evaluate(&p, &purpose("urn:purpose/b")).allow);
    assert!(!evaluate(&p, &purpose("urn:purpose/c")).allow);
}

// ===========================================================================
// #3982 — a NESTED-list member of a rightOperand collection REFUSES the parse.
//
// A nested list used to stay an opaque blank-node string inside the set
// encoding: unmatchable, which is fail-closed only for the POSITIVE set
// operators. Under `isNoneOf` an unmatchable member excludes nothing, so the
// authored exclusion was silently dropped (widening on a permission; a
// narrowed carve-out on a prohibition). Mirrors `fold_list_operands`.
// ===========================================================================

/// The issue's witness: `isNoneOf ( ( <a> ) <b> )` on a permission used to
/// GRANT purpose `a`, which the author listed in the exclusion set.
#[test]
fn nested_list_member_right_operand_refuses_the_parse_on_permission() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isNoneOf ;
                      odrl:rightOperand ( ( <urn:purpose/a> ) <urn:purpose/b> ) ] ] .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("NESTED-list member"),
        "a nested-list rightOperand member must refuse the parse, got: {err}"
    );
}

/// The same shape gating a PROHIBITION (where an unmatchable member would
/// narrow the carve-out) is refused too.
#[test]
fn nested_list_member_right_operand_refuses_the_parse_on_prohibition() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isAnyOf ;
                      odrl:rightOperand ( <urn:purpose/b> ( <urn:purpose/a> ) ) ] ] .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("NESTED-list member"),
        "a nested-list rightOperand member must refuse the parse, got: {err}"
    );
}

/// An EMPTY nested list (`( )` as a member, i.e. `rdf:nil`) is a degenerate
/// nested collection and is refused the same way.
#[test]
fn empty_nested_list_member_right_operand_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isNoneOf ;
                      odrl:rightOperand ( ( ) <urn:purpose/b> ) ] ] .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(err.contains("NESTED-list member"), "got: {err}");
}

/// Nested lists inside a LogicalConstraint's atomic operand are refused too
/// (the atom table shares the same fold).
#[test]
fn nested_list_member_in_logical_operand_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:and
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:isNoneOf ;
          odrl:rightOperand ( ( <urn:purpose/a> ) <urn:purpose/b> ) ] ,
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:neq ;
          odrl:rightOperand <urn:purpose/c> ] ] ] .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(err.contains("NESTED-list member"), "got: {err}");
}

// ===========================================================================
// #3832 — a constraint node with SEVERAL distinct `odrl:leftOperand` or
// `odrl:operator` objects REFUSES the parse instead of evaluating one
// arbitrarily chosen binding (unstable under result ordering).
// ===========================================================================

#[test]
fn multiple_left_operands_refuse_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient , odrl:purpose ;
                      odrl:operator odrl:eq ; odrl:rightOperand <urn:alice> ] ] .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("several distinct odrl:leftOperand"),
        "got: {err}"
    );
}

#[test]
fn multiple_operators_refuse_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ;
                      odrl:operator odrl:eq , odrl:neq ; odrl:rightOperand <urn:purpose/a> ] ] .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(err.contains("several distinct odrl:operator"), "got: {err}");
}

/// The same refusal applies to an atomic operand of a LogicalConstraint.
#[test]
fn multiple_left_operands_in_logical_operand_refuse_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:or
        [ odrl:leftOperand odrl:recipient , odrl:purpose ;
          odrl:operator odrl:eq ; odrl:rightOperand <urn:alice> ] ,
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
          odrl:rightOperand <urn:purpose/c> ] ] ] .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("several distinct odrl:leftOperand"),
        "got: {err}"
    );
}

/// A single leftOperand repeated across several rows (one per rightOperand
/// object) is NOT a conflict — the multi-object fold still works.
#[test]
fn repeated_identical_left_operand_rows_still_fold() {
    let p = purpose_policy("isNoneOf", "<urn:purpose/a>, <urn:purpose/b>");
    assert!(!evaluate(&p, &purpose("urn:purpose/a")).allow);
    assert!(!evaluate(&p, &purpose("urn:purpose/b")).allow);
    assert!(evaluate(&p, &purpose("urn:purpose/c")).allow);
}
