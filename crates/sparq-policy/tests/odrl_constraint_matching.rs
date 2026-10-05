//! ODRL constraint-matching conformance batch — end-to-end parse + evaluate tests
//! for the three related matching-semantics features, each through the REAL
//! `parse_policy_str` + `evaluate` path (not a mock). [OPUS-4.8]
//!
//! - **`odrl:use` umbrella vs the ODRL action hierarchy (sq-euhr3).** `use` subsumes
//!   its sub-actions (`read`/`write`/…) but NOT the ownership-`transfer` subtree
//!   (`sell`/`give`) — the canonical ODRL 2.2 reading the SolidLab suite expects.
//! - **`odrl:PartyCollection`/`odrl:AssetCollection` membership (sq-k7itg).** A rule
//!   targeting a collection matches a request whose party/asset is `odrl:partOf` it.
//! - **`odrl:LogicalConstraint` compound constraints (sq-a0zef).** `odrl:and`/`or`/
//!   `xone` over a (possibly nested) operand set, fail-closed on unprovable operands.

use sparq_policy::{evaluate, parse_policy_str, Request, Value};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

fn left(local: &str) -> String {
    format!("{ODRL}{local}")
}
fn dt(s: &str) -> Value {
    Value::DateTime(s.to_owned())
}

// ===========================================================================
// sq-euhr3 — odrl:use umbrella reconciled with the ODRL 2.2 action hierarchy.
// ===========================================================================

/// A `use` permission permits its included sub-actions (read/write) but NOT the
/// disjoint `transfer` subtree (sell/give) — matching SolidLab cases 007/009 (Active)
/// vs 010/017 (Inactive).
#[test]
fn use_subsumes_sub_actions_but_not_transfer_subtree() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/u> a odrl:Set ; odrl:permission [
    odrl:action odrl:use ; odrl:target <urn:asset/x> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // read, write, display → permitted (included in `use`).
    for act in ["read", "write", "display", "modify", "print"] {
        let req = Request::new(left(act)).on("urn:asset/x");
        assert!(evaluate(&p, &req).allow, "use should permit {act}");
    }
    // sell, give, transfer → DENIED (the ownership-transfer subtree is outside `use`).
    for act in ["sell", "give", "transfer"] {
        let req = Request::new(left(act)).on("urn:asset/x");
        assert!(
            !evaluate(&p, &req).allow,
            "use must NOT permit the transfer-subtree action {act}"
        );
    }
}

/// A concrete (non-`use`) permission still matches only its own action exactly.
#[test]
fn concrete_action_is_exact() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/r> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert!(evaluate(&p, &Request::new(left("read")).on("urn:asset/x")).allow);
    // a `read` permission does not cover `write`/`sell`.
    assert!(!evaluate(&p, &Request::new(left("write")).on("urn:asset/x")).allow);
    assert!(!evaluate(&p, &Request::new(left("sell")).on("urn:asset/x")).allow);
}

/// A `use` PROHIBITION carves out its sub-actions but not the transfer subtree, so a
/// `sell` request is not blocked by a `use` prohibition (consistency with the grant
/// path through the deny-overrides carve-out).
#[test]
fn use_prohibition_carves_sub_actions_not_transfer() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
    odrl:permission [ odrl:action odrl:use ; odrl:target <urn:asset/x> ] ;
    odrl:prohibition [ odrl:action odrl:use ; odrl:target <urn:asset/x> ;
        odrl:assignee <https://bob.ex/me> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // bob asks to read → the use prohibition carves him out → DENY.
    let read = Request::new(left("read"))
        .on("urn:asset/x")
        .by("https://bob.ex/me");
    assert!(!evaluate(&p, &read).allow);
    // bob asks to sell → outside `use`; the prohibition does NOT carve it out, and the
    // `use` permission also does not grant it → DENY (no permission), but NOT via the
    // prohibition (the request is not in the use subtree at all).
    let sell = Request::new(left("sell"))
        .on("urn:asset/x")
        .by("https://bob.ex/me");
    assert!(!evaluate(&p, &sell).allow);
}

// ===========================================================================
// sq-k7itg — Party/Asset collection membership matching.
// ===========================================================================

/// A permission whose `assignee` is an `odrl:PartyCollection` grants a request whose
/// party is `odrl:partOf` that collection (membership evidence supplied per the sotw).
#[test]
fn party_collection_membership_grants_member() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .
<urn:pol/pc> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target ex:x ; odrl:assignee ex:partyCollection ] .
ex:partyCollection a odrl:PartyCollection ; odrl:source ex:partyIdentifier .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // alice IS a member → grant.
    let member = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/alice")
        .with_party_membership(
            "http://example.org/alice",
            "http://example.org/partyCollection",
        );
    assert!(
        evaluate(&p, &member).allow,
        "a collection member must be granted"
    );

    // carol is NOT a member (no membership edge) → DENY (fail-closed: not widened).
    let nonmember = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/carol");
    assert!(
        !evaluate(&p, &nonmember).allow,
        "a non-member must be denied (membership is never inferred)"
    );
}

/// A permission whose `target` is an `odrl:AssetCollection` grants a request whose asset
/// is `odrl:partOf` that collection.
#[test]
fn asset_collection_membership_grants_member_asset() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .
<urn:pol/ac> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target ex:assetCollection ; odrl:assignee ex:alice ] .
ex:assetCollection a odrl:AssetCollection ; odrl:source ex:assetIdentifier .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let in_collection = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/alice")
        .with_asset_membership("http://example.org/x", "http://example.org/assetCollection");
    assert!(evaluate(&p, &in_collection).allow);

    // an asset NOT in the collection → DENY.
    let outside = Request::new(left("read"))
        .on("http://example.org/y")
        .by("http://example.org/alice");
    assert!(!evaluate(&p, &outside).allow);
}

/// Both collections at once (the suite's 055 shape): party-in-collection AND
/// asset-in-collection.
#[test]
fn both_collections_compose() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .
<urn:pol/both> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target ex:assetCollection ; odrl:assignee ex:partyCollection ] .
ex:partyCollection a odrl:PartyCollection ; odrl:source ex:partyIdentifier .
ex:assetCollection a odrl:AssetCollection ; odrl:source ex:assetIdentifier .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let req = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/alice")
        .with_party_memberships([(
            "http://example.org/alice",
            "http://example.org/partyCollection",
        )])
        .with_asset_memberships([("http://example.org/x", "http://example.org/assetCollection")]);
    assert!(evaluate(&p, &req).allow);

    // missing the party membership → DENY (one half short).
    let req_no_party = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/alice")
        .with_asset_membership("http://example.org/x", "http://example.org/assetCollection");
    assert!(!evaluate(&p, &req_no_party).allow);
}

/// Membership for the WRONG collection does not match (the edge must name the rule's
/// collection, not just any collection).
#[test]
fn membership_for_wrong_collection_does_not_match() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .
<urn:pol/pc> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target ex:x ; odrl:assignee ex:groupA ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let req = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/alice")
        // alice is in groupB, but the rule names groupA.
        .with_party_membership("http://example.org/alice", "http://example.org/groupB");
    assert!(!evaluate(&p, &req).allow);
}

// ===========================================================================
// sq-a0zef — LogicalConstraint (and / or / xone) compound constraints.
// ===========================================================================

/// `odrl:and` of two dateTime bounds (a closed time window) — satisfied inside the
/// window, definitely-unsatisfied outside, fail-closed with no time evidence.
#[test]
fn logical_and_two_sided_window() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/and> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:and
        [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;
          odrl:rightOperand "2024-01-01T00:00:00Z"^^xsd:dateTime ] ,
        [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
          odrl:rightOperand "2024-12-31T23:59:59Z"^^xsd:dateTime ] ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert_eq!(p.permissions[0].logical_constraints.len(), 1);
    let base = Request::new(left("read")).on("urn:asset/x");
    // inside the window → both operands hold → AND satisfied → grant.
    assert!(evaluate(&p, &base.clone().at("2024-06-15T12:00:00Z")).allow);
    // before the window → the `gt` operand definitely fails → AND fails → deny.
    assert!(!evaluate(&p, &base.clone().at("2023-12-31T00:00:00Z")).allow);
    // after the window → the `lt` operand definitely fails → deny.
    assert!(!evaluate(&p, &base.clone().at("2025-06-01T00:00:00Z")).allow);
    // NO time evidence → unprovable → fail-closed (no silent pass on an AND).
    assert!(!evaluate(&p, &base).allow);
}

/// `odrl:or` — satisfied iff at least one operand holds; definitely-unsatisfied iff every
/// operand definitely fails.
#[test]
fn logical_or_at_least_one() {
    // Permit if the purpose is research OR teaching.
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/or> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
          odrl:rightOperand <urn:purpose/research> ] ,
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
          odrl:rightOperand <urn:purpose/teaching> ] ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let base = Request::new(left("read")).on("urn:asset/x");
    // research → one operand holds → grant.
    let research = base
        .clone()
        .for_purpose(Value::Iri("urn:purpose/research".into()));
    assert!(evaluate(&p, &research).allow);
    // teaching → the other operand holds → grant.
    let teaching = base
        .clone()
        .for_purpose(Value::Iri("urn:purpose/teaching".into()));
    assert!(evaluate(&p, &teaching).allow);
    // marketing → both operands definitely fail → deny.
    let marketing = base
        .clone()
        .for_purpose(Value::Iri("urn:purpose/marketing".into()));
    assert!(!evaluate(&p, &marketing).allow);
    // no purpose → both unprovable → fail-closed (no grant).
    assert!(!evaluate(&p, &base).allow);
}

/// `odrl:xone` — satisfied iff EXACTLY ONE operand holds; an unprovable operand keeps the
/// exact-one count from being provable (fail-closed).
#[test]
fn logical_xone_exactly_one() {
    // Two disjoint purpose options; exactly one must hold.
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/xone> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:xone
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
          odrl:rightOperand <urn:purpose/a> ] ,
        [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
          odrl:rightOperand <https://bob.ex/me> ] ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // purpose=a, recipient!=bob → exactly one holds → grant.
    let one = Request::new(left("read"))
        .on("urn:asset/x")
        .by("https://carol.ex/me")
        .for_purpose(Value::Iri("urn:purpose/a".into()));
    assert!(evaluate(&p, &one).allow);
    // purpose=a AND recipient=bob → TWO hold → xone fails → deny.
    let both = Request::new(left("read"))
        .on("urn:asset/x")
        .by("https://bob.ex/me")
        .for_purpose(Value::Iri("urn:purpose/a".into()));
    assert!(!evaluate(&p, &both).allow);
    // purpose=other, recipient=bob → exactly one (recipient) holds → grant.
    let other_one = Request::new(left("read"))
        .on("urn:asset/x")
        .by("https://bob.ex/me")
        .for_purpose(Value::Iri("urn:purpose/other".into()));
    assert!(evaluate(&p, &other_one).allow);
    // purpose missing (unprovable) + recipient=bob (holds): the count is not provably
    // exactly-one (the purpose operand could also hold) → fail-closed deny.
    let unprovable = Request::new(left("read"))
        .on("urn:asset/x")
        .by("https://bob.ex/me");
    assert!(!evaluate(&p, &unprovable).allow);
}

/// A NESTED LogicalConstraint — an `odrl:or` of `odrl:and`s (the suite's 062 shape:
/// a disjunction of time windows). Satisfied iff the request instant falls in any window.
#[test]
fn nested_or_of_ands_time_windows() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/nested> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or
        [ a odrl:LogicalConstraint ; odrl:and
            [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;
              odrl:rightOperand "2024-01-01T09:00:00Z"^^xsd:dateTime ] ,
            [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
              odrl:rightOperand "2024-01-01T17:00:00Z"^^xsd:dateTime ] ] ,
        [ a odrl:LogicalConstraint ; odrl:and
            [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;
              odrl:rightOperand "2024-02-12T09:00:00Z"^^xsd:dateTime ] ,
            [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
              odrl:rightOperand "2024-02-12T17:00:00Z"^^xsd:dateTime ] ] ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let base = Request::new(left("read")).on("urn:asset/x");
    // 2024-02-12T11:20 → falls in the SECOND window → grant.
    assert!(evaluate(&p, &base.clone().at("2024-02-12T11:20:10Z")).allow);
    // 2024-01-01T12:00 → falls in the FIRST window → grant.
    assert!(evaluate(&p, &base.clone().at("2024-01-01T12:00:00Z")).allow);
    // 2024-03-01T12:00 → in NEITHER window → both AND-windows definitely fail → deny.
    assert!(!evaluate(&p, &base.clone().at("2024-03-01T12:00:00Z")).allow);
    // no time at all → both windows unprovable → fail-closed.
    assert!(!evaluate(&p, &base).allow);
}

/// A LogicalConstraint AND a sibling atomic constraint on one rule are themselves ANDed:
/// the rule grants only when both the compound and the atomic constraint hold.
#[test]
fn logical_and_atomic_constraints_are_anded() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/mix> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
        odrl:rightOperand <urn:purpose/research> ] ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or
        [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
          odrl:rightOperand "2024-12-31T23:59:59Z"^^xsd:dateTime ] ,
        [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;
          odrl:rightOperand "2030-01-01T00:00:00Z"^^xsd:dateTime ] ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert_eq!(
        p.permissions[0].constraints.len(),
        1,
        "one atomic constraint"
    );
    assert_eq!(
        p.permissions[0].logical_constraints.len(),
        1,
        "one compound"
    );
    let base = Request::new(left("read")).on("urn:asset/x");
    // purpose=research AND time<2024-end (the OR's first branch) → grant.
    let ok = base
        .clone()
        .for_purpose(Value::Iri("urn:purpose/research".into()))
        .with(left("dateTime"), dt("2024-06-01T00:00:00Z"));
    assert!(evaluate(&p, &ok).allow);
    // right time but WRONG purpose → the atomic constraint fails → deny.
    let wrong_purpose = base
        .clone()
        .for_purpose(Value::Iri("urn:purpose/marketing".into()))
        .with(left("dateTime"), dt("2024-06-01T00:00:00Z"));
    assert!(!evaluate(&p, &wrong_purpose).allow);
    // right purpose but time in NEITHER OR branch → the compound fails → deny.
    let wrong_time = base
        .clone()
        .for_purpose(Value::Iri("urn:purpose/research".into()))
        .with(left("dateTime"), dt("2027-06-01T00:00:00Z"));
    assert!(!evaluate(&p, &wrong_time).allow);
}

/// A LogicalConstraint that gates a PROHIBITION carves out the request only when the
/// compound holds (deny-overrides), and the carve-out lifts when the compound fails.
#[test]
fn logical_constraint_on_prohibition() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/p> a odrl:Set ;
    odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
        odrl:assignee <https://alice.ex/me> ] ;
    odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
        odrl:assignee <https://alice.ex/me> ;
        odrl:constraint [ a odrl:LogicalConstraint ; odrl:and
            [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;
              odrl:rightOperand "2024-01-01T00:00:00Z"^^xsd:dateTime ] ,
            [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
              odrl:rightOperand "2024-12-31T23:59:59Z"^^xsd:dateTime ] ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let base = Request::new(left("read"))
        .on("urn:asset/x")
        .by("https://alice.ex/me");
    // inside the prohibited window → the compound holds → carve-out fires → DENY.
    assert!(!evaluate(&p, &base.clone().at("2024-06-01T00:00:00Z")).allow);
    // outside the window → the compound fails → carve-out gone → the permission grants.
    assert!(evaluate(&p, &base.clone().at("2025-06-01T00:00:00Z")).allow);
}

/// An empty `odrl:or` (a LogicalConstraint with no operand) can never be satisfied —
/// fail-closed. (A malformed compound must not silently pass.)
#[test]
fn empty_or_is_fail_closed() {
    // An `or` whose only operand is a malformed (unknown-operator) atomic constraint →
    // that operand is the unsatisfiable guard → the `or` definitely fails → deny.
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/bad> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:bogusOp ;
          odrl:rightOperand <urn:purpose/x> ] ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .for_purpose(Value::Iri("urn:purpose/x".into()));
    assert!(
        !evaluate(&p, &req).allow,
        "a malformed compound operand must fail closed"
    );
}

// ===========================================================================
// sq-c2aze — `odrl:recipient` constraints resolve party-collection membership
// ([FABLE-5]): a recipient may be a party OR a member of an `odrl:PartyCollection`,
// mirroring the assignee field's equality-or-membership lookup. Membership draws
// ONLY on the request-supplied `with_party_membership(s)` evidence — with no edge,
// recipient matching stays the flat base case (fail-closed, never widened).
// ===========================================================================

/// The bead's headline case: a `recipient isPartOf <PartyCollectionIRI>` constraint
/// is satisfied by a *member* of that collection (previously the flat string split
/// could only match the collection IRI itself).
#[test]
fn recipient_is_part_of_party_collection_grants_member() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .
<urn:pol/rc> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target ex:x ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand ex:team ] ] .
ex:team a odrl:PartyCollection .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // alice IS a member of ex:team → the recipient constraint is satisfied → grant.
    let member = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/alice")
        .with_party_membership("http://example.org/alice", "http://example.org/team");
    assert!(
        evaluate(&p, &member).allow,
        "a recipient who is a member of the party collection must be granted"
    );
    // bob supplies membership evidence — but in a DIFFERENT collection → DENY.
    let nonmember = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/bob")
        .with_party_membership("http://example.org/bob", "http://example.org/otherTeam");
    assert!(
        !evaluate(&p, &nonmember).allow,
        "membership in a different collection must not match"
    );
    // NO membership evidence at all → the flat base case → DENY (never widened).
    let no_evidence = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/alice");
    assert!(
        !evaluate(&p, &no_evidence).allow,
        "without membership evidence the base case is unchanged (fail-closed)"
    );
}

/// `recipient eq <collection>` also resolves membership — the exact
/// equality-or-membership shape the assignee field gets via `party_matches`.
#[test]
fn recipient_eq_collection_matches_member() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .
<urn:pol/re> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target ex:x ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:eq ;
                      odrl:rightOperand ex:team ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let member = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/alice")
        .with_party_membership("http://example.org/alice", "http://example.org/team");
    assert!(evaluate(&p, &member).allow);
    let outsider = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/eve")
        .with_party_membership("http://example.org/eve", "http://example.org/otherTeam");
    assert!(!evaluate(&p, &outsider).allow);
}

/// `recipient neq <collection>` EXCLUDES a member of that collection (the negative
/// dual — the carve-out extends to members, mirroring the taxonomic `neq`; being in
/// the excluded group is being the excluded recipient).
#[test]
fn recipient_neq_collection_excludes_members() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .
<urn:pol/rn> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target ex:x ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
                      odrl:rightOperand ex:blocked ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // A member of the excluded collection is ALSO excluded (no widening away).
    let blocked_member = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/mallory")
        .with_party_membership("http://example.org/mallory", "http://example.org/blocked");
    assert!(
        !evaluate(&p, &blocked_member).allow,
        "a member of the excluded collection must be denied"
    );
    // A party whose membership evidence names an UNRELATED collection is not excluded.
    let outsider = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/carol")
        .with_party_membership("http://example.org/carol", "http://example.org/team");
    assert!(evaluate(&p, &outsider).allow);
}

/// A PROHIBITION whose recipient constraint names a collection carves out its
/// members (deny-overrides through the same membership resolution).
#[test]
fn recipient_prohibition_carves_out_collection_members() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .
<urn:pol/rp> a odrl:Set ;
    odrl:permission  [ odrl:action odrl:read ; odrl:target ex:x ] ;
    odrl:prohibition [ odrl:action odrl:read ; odrl:target ex:x ;
        odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:isPartOf ;
                          odrl:rightOperand ex:blocked ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let blocked_member = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/mallory")
        .with_party_membership("http://example.org/mallory", "http://example.org/blocked");
    assert!(
        !evaluate(&p, &blocked_member).allow,
        "the prohibition must carve out a member of the blocked collection"
    );
    let outsider = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/carol")
        .with_party_membership("http://example.org/carol", "http://example.org/team");
    assert!(
        evaluate(&p, &outsider).allow,
        "a non-member is not carved out"
    );
}

/// An EXPLICIT `odrl:recipient` context value (the disclosure target need not be
/// the requester) resolves through the same membership evidence.
#[test]
fn explicit_recipient_context_resolves_via_membership() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .
<urn:pol/rx> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target ex:x ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand ex:team ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // alice asks, disclosing to dave — dave (the explicit recipient) is the member.
    let req = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/alice")
        .with(
            "http://www.w3.org/ns/odrl/2/recipient",
            Value::Iri("http://example.org/dave".into()),
        )
        .with_party_membership("http://example.org/dave", "http://example.org/team");
    assert!(evaluate(&p, &req).allow);
    // alice's OWN membership does not stand in for the explicit recipient's.
    let wrong = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/alice")
        .with(
            "http://www.w3.org/ns/odrl/2/recipient",
            Value::Iri("http://example.org/dave".into()),
        )
        .with_party_membership("http://example.org/alice", "http://example.org/team");
    assert!(!evaluate(&p, &wrong).allow);
}

/// The two beads compose: `recipient isNoneOf "<g1>|<g2>"` (sq-uaz85) excludes a
/// MEMBER of g1 through the membership resolution (sq-c2aze); an unrelated party
/// with membership evidence in another group still grants.
#[test]
fn recipient_is_none_of_excludes_collection_members() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .
<urn:pol/rno> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target ex:x ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:isNoneOf ;
        odrl:rightOperand "http://example.org/g1|http://example.org/g2" ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let g1_member = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/mallory")
        .with_party_membership("http://example.org/mallory", "http://example.org/g1");
    assert!(
        !evaluate(&p, &g1_member).allow,
        "a member of an excluded collection must be denied under isNoneOf"
    );
    let outsider = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/carol")
        .with_party_membership("http://example.org/carol", "http://example.org/team");
    assert!(evaluate(&p, &outsider).allow);
}

/// The `recipient_status` audit surface reports the membership-resolved verdict —
/// exactly what the evaluator acts on (member → Satisfied; non-member →
/// DefinitelyUnsatisfied; no identity → Unprovable).
#[test]
fn recipient_status_reflects_membership_resolution() {
    use sparq_policy::{recipient_status, RecipientMatch};
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .
<urn:pol/rs> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target ex:x ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand ex:team ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let rule = &p.permissions[0];
    let member = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/alice")
        .with_party_membership("http://example.org/alice", "http://example.org/team");
    assert_eq!(recipient_status(rule, &member), RecipientMatch::Satisfied);
    let nonmember = Request::new(left("read"))
        .on("http://example.org/x")
        .by("http://example.org/bob")
        .with_party_membership("http://example.org/bob", "http://example.org/otherTeam");
    assert_eq!(
        recipient_status(rule, &nonmember),
        RecipientMatch::DefinitelyUnsatisfied
    );
    let anonymous = Request::new(left("read")).on("http://example.org/x");
    assert_eq!(
        recipient_status(rule, &anonymous),
        RecipientMatch::Unprovable
    );
}

// ===========================================================================
// sq-dkuff — LIST-valued LogicalConstraint combinator operands ([FABLE-5]):
// `odrl:or ( <c1> <c2> )` binds the combinator object to the RDF-collection
// HEAD; pre-fold that head degraded to the unsatisfiable guard (fail-closed on
// permissions, but silently DISABLING a prohibition's compound carve-out). The
// head is now expanded into its member constraints before assembly.
// ===========================================================================

/// `odrl:or` with a LIST-valued operand set parses to the member constraints and
/// evaluates faithfully (grant on either member purpose, deny otherwise).
#[test]
fn or_list_valued_operand_set() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/orlist> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or (
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
          odrl:rightOperand <urn:purpose/research> ]
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
          odrl:rightOperand <urn:purpose/teaching> ] ) ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let lcs = &p.permissions[0].logical_constraints;
    assert_eq!(lcs.len(), 1, "the compound must parse");
    // Structural witness: the LIST head must be replaced by its TWO member
    // constraints (pre-fold this was ONE unsatisfiable-guard operand).
    assert_eq!(
        lcs[0].operands.len(),
        2,
        "the list head must expand to its member operands, got {:?}",
        lcs[0].operands
    );
    let base = Request::new(left("read")).on("urn:asset/x");
    for purpose in ["urn:purpose/research", "urn:purpose/teaching"] {
        let req = base.clone().for_purpose(Value::Iri(purpose.into()));
        assert!(evaluate(&p, &req).allow, "OR member {purpose} must grant");
    }
    // Non-member purpose → both operands definitely fail → deny.
    let marketing = base
        .clone()
        .for_purpose(Value::Iri("urn:purpose/marketing".into()));
    assert!(!evaluate(&p, &marketing).allow);
    // No purpose evidence → unprovable → fail-closed.
    assert!(!evaluate(&p, &base).allow);
}

/// `odrl:and` with a LIST-valued operand set (a closed time window) — inside the
/// window grants, outside/unprovable denies.
#[test]
fn and_list_valued_time_window() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/andlist> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:and (
        [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;
          odrl:rightOperand "2024-01-01T00:00:00Z"^^xsd:dateTime ]
        [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
          odrl:rightOperand "2024-12-31T23:59:59Z"^^xsd:dateTime ] ) ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert_eq!(p.permissions[0].logical_constraints[0].operands.len(), 2);
    let base = Request::new(left("read")).on("urn:asset/x");
    assert!(evaluate(&p, &base.clone().at("2024-06-15T12:00:00Z")).allow);
    assert!(!evaluate(&p, &base.clone().at("2025-06-01T00:00:00Z")).allow);
    assert!(!evaluate(&p, &base).allow, "no time evidence → fail-closed");
}

/// THE widening-hazard case the fold closes: a PROHIBITION whose compound uses a
/// LIST-valued `odrl:and`. Pre-fold the head degraded to an unsatisfiable operand,
/// the carve-out never fired, and the sibling permission granted INSIDE the
/// prohibited window (deny-overrides silently bypassed).
#[test]
fn prohibition_list_valued_and_fires_inside_window() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/plist> a odrl:Set ;
    odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
    odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
        odrl:constraint [ a odrl:LogicalConstraint ; odrl:and (
            [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;
              odrl:rightOperand "2024-01-01T00:00:00Z"^^xsd:dateTime ]
            [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
              odrl:rightOperand "2024-12-31T23:59:59Z"^^xsd:dateTime ] ) ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let base = Request::new(left("read")).on("urn:asset/x");
    // Inside the prohibited window → the compound holds → DENY (pre-fold: allow).
    assert!(
        !evaluate(&p, &base.clone().at("2024-06-01T00:00:00Z")).allow,
        "a list-valued prohibition compound must fire inside its window"
    );
    // Outside the window → the carve-out lifts → the permission grants.
    assert!(evaluate(&p, &base.clone().at("2025-06-01T00:00:00Z")).allow);
}

/// Mixed operand forms on ONE combinator — a direct object AND a list — merge
/// (in order, deduplicated) into one operand set.
#[test]
fn mixed_direct_and_list_operands_merge() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/mixed> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ;
        odrl:or _:c1 ;
        odrl:or ( _:c2 ) ] ] .
_:c1 odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
     odrl:rightOperand <urn:purpose/research> .
_:c2 odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
     odrl:rightOperand <urn:purpose/teaching> .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let lcs = &p.permissions[0].logical_constraints;
    assert_eq!(lcs.len(), 1);
    assert_eq!(lcs[0].operands.len(), 2, "direct + list member must merge");
    let base = Request::new(left("read")).on("urn:asset/x");
    for purpose in ["urn:purpose/research", "urn:purpose/teaching"] {
        let req = base.clone().for_purpose(Value::Iri(purpose.into()));
        assert!(
            evaluate(&p, &req).allow,
            "merged member {purpose} must grant"
        );
    }
    let other = base.for_purpose(Value::Iri("urn:purpose/marketing".into()));
    assert!(!evaluate(&p, &other).allow);
}

/// A list member that is itself a NESTED compound `odrl:LogicalConstraint`
/// recurses through the normal compound assembly (an `odrl:or` of a listed
/// `odrl:and` window).
#[test]
fn list_member_nested_compound_recurses() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/nestedlist> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or (
        [ a odrl:LogicalConstraint ; odrl:and (
            [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;
              odrl:rightOperand "2024-01-01T09:00:00Z"^^xsd:dateTime ]
            [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
              odrl:rightOperand "2024-01-01T17:00:00Z"^^xsd:dateTime ] ) ]
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
          odrl:rightOperand <urn:purpose/research> ] ) ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let base = Request::new(left("read")).on("urn:asset/x");
    // In the nested window → the compound member holds → grant.
    assert!(evaluate(&p, &base.clone().at("2024-01-01T12:00:00Z")).allow);
    // Outside the window but the research purpose → the atomic member holds → grant.
    let research = base
        .clone()
        .at("2025-01-01T12:00:00Z")
        .for_purpose(Value::Iri("urn:purpose/research".into()));
    assert!(evaluate(&p, &research).allow);
    // Neither member holds → deny.
    let neither = base
        .at("2025-01-01T12:00:00Z")
        .for_purpose(Value::Iri("urn:purpose/marketing".into()));
    assert!(!evaluate(&p, &neither).allow);
}

/// An EMPTY list operand (`odrl:or ()` — `rdf:nil` directly) REFUSES the whole
/// parse: per-operand degradation would silently DISABLE a prohibition's compound
/// carve-out (deny-overrides bypassed), so the degenerate shape is rejected
/// outright (fail-closed on both rule kinds).
#[test]
fn empty_list_operand_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/emptylist> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or () ] ] .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("EMPTY collection operand"),
        "an empty combinator collection must refuse the parse, got: {err}"
    );
}

/// A NESTED-list member (a list inside the list) REFUSES the whole parse — one
/// expansion level only, mirroring the rightOperand fold; silently flattening or
/// degrading could mis-honour the authored structure on either rule kind.
#[test]
fn nested_list_member_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/nestednil> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:and (
        ( [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
            odrl:rightOperand <urn:purpose/x> ] ) ) ] ] .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("NESTED-list member"),
        "a nested-list member must refuse the parse, got: {err}"
    );
}

/// A MALFORMED collection (broken tail — a cons cell with `rdf:first` but no
/// `rdf:rest`) REFUSES the parse: honouring the valid PREFIX of an `odrl:and`
/// operand list would make the compound EASIER to satisfy than authored
/// (widening). Written with explicit cons-cell triples (Turtle `( … )` sugar
/// always emits well-formed lists).
#[test]
fn broken_tail_list_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/broken> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:and _:l1 ] ] .
_:l1 rdf:first _:c1 .
_:c1 odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
     odrl:rightOperand <urn:purpose/x> .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection operand"),
        "a broken-tail list must refuse the parse, got: {err}"
    );
}

/// A CYCLIC collection (`rdf:rest` looping back to the head) REFUSES the parse —
/// a cycle never reaches `rdf:nil`, so the collection has no well-defined member
/// set. Crucially this holds on a PROHIBITION too: a degraded compound would
/// silently disable the carve-out and let the sibling permission grant (widening).
#[test]
fn cyclic_list_on_prohibition_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/cyclic> a odrl:Set ;
    odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
    odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
        odrl:constraint [ a odrl:LogicalConstraint ; odrl:and _:l1 ] ] .
_:l1 rdf:first _:c1 ; rdf:rest _:l1 .
_:c1 odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
     odrl:rightOperand <urn:purpose/x> .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection operand"),
        "a cyclic list (esp. gating a prohibition) must refuse the parse, got: {err}"
    );
}

/// A FORKED cons cell (two distinct `rdf:rest` values) REFUSES the parse:
/// honouring one deterministic fork of an ambiguous collection could silently
/// drop authored members.
#[test]
fn forked_list_cell_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/forked> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:and _:l1 ] ] .
_:l1 rdf:first _:c1 ; rdf:rest _:l2 , rdf:nil .
_:l2 rdf:first _:c2 ; rdf:rest rdf:nil .
_:c1 odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
     odrl:rightOperand <urn:purpose/x> .
_:c2 odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
     odrl:rightOperand <urn:purpose/y> .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection operand"),
        "a forked list cell must refuse the parse, got: {err}"
    );
}

/// Regression: the direct multi-object combinator form (`odrl:or <c1>, <c2>` — the
/// SolidLab suite's form) is untouched by the list fold.
#[test]
fn direct_object_operands_unchanged_by_list_fold() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/direct> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
          odrl:rightOperand <urn:purpose/research> ] ,
        [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
          odrl:rightOperand <urn:purpose/teaching> ] ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert_eq!(p.permissions[0].logical_constraints[0].operands.len(), 2);
    let req = Request::new(left("read"))
        .on("urn:asset/x")
        .for_purpose(Value::Iri("urn:purpose/teaching".into()));
    assert!(evaluate(&p, &req).allow);
}

/// A HEAD-position rest-only cons cell (`rdf:rest` but NO `rdf:first`) is
/// invisible to the `rdf:first`-keyed cells table, so it would silently bypass
/// collection validation and degrade — disabling a PROHIBITION's compound
/// (widening). It REFUSES the parse instead.
#[test]
fn rest_only_head_on_prohibition_refuses_the_parse() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/restonly> a odrl:Set ;
    odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
    odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
        odrl:constraint [ a odrl:LogicalConstraint ; odrl:and _:l1 ] ] .
_:l1 rdf:rest rdf:nil .
"#;
    let err = parse_policy_str(ttl, "turtle").unwrap_err();
    assert!(
        err.contains("MALFORMED collection operand"),
        "a rest-only head cell must refuse the parse, got: {err}"
    );
}

/// Constraint-reading PRECEDENCE: a combinator operand that is a real atomic
/// constraint keeps that reading even when the node is `rdf:nil` (pathological,
/// but previously accepted — the refusal paths must stay strictly narrow).
#[test]
fn nil_with_constraint_reading_keeps_precedence() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<urn:pol/nilatom> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or rdf:nil ] ] .
rdf:nil odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
    odrl:rightOperand <urn:purpose/x> .
"#;
    let p = parse_policy_str(ttl, "turtle")
        .expect("a nil node WITH a constraint reading must keep it, not refuse");
    let base = Request::new(left("read")).on("urn:asset/x");
    let ok = base.clone().for_purpose(Value::Iri("urn:purpose/x".into()));
    assert!(evaluate(&p, &ok).allow, "the atomic reading must evaluate");
    let wrong = base.for_purpose(Value::Iri("urn:purpose/y".into()));
    assert!(!evaluate(&p, &wrong).allow);
}

// ===========================================================================
// sq-rf9uv — `Request::party_collection_members`: the public READ side of the
// membership evidence, added so a consumer that PERSISTS a rule as a re-checked head
// (the sparq-solid ODRL bridge) can expand a collection-valued `odrl:assignee` into one
// head per member — `party_matches` is unusable there because a session carries no
// membership evidence. [SONNET-4.6]
// ===========================================================================

#[test]
fn party_collection_members_reports_exactly_the_supplied_evidence() {
    let req = Request::new(left("read")).on("http://example.org/x").with_party_memberships([
        ("http://example.org/alice", "http://example.org/team"),
        ("http://example.org/bob", "http://example.org/team"),
        ("http://example.org/eve", "http://example.org/other"),
    ]);

    // Members of a collection — deterministic sorted order, only that collection.
    assert_eq!(
        req.party_collection_members("http://example.org/team"),
        vec!["http://example.org/alice", "http://example.org/bob"],
        "both team members, and NOT the member of the other collection"
    );
    assert_eq!(req.party_collection_members("http://example.org/other"), vec!["http://example.org/eve"]);

    // A collection with no supplied evidence, and a plain party IRI, are both empty —
    // membership is never inferred from IRI structure or from the reverse edge.
    assert!(req.party_collection_members("http://example.org/unknown").is_empty());
    assert!(
        req.party_collection_members("http://example.org/alice").is_empty(),
        "the edge is directional: alice is a member, not a collection"
    );

    // A request that supplied NO evidence reports no members for anything (the base
    // case a consumer must fall back to — never widened on absent evidence).
    let bare = Request::new(left("read")).on("http://example.org/x");
    assert!(bare.party_collection_members("http://example.org/team").is_empty());
}

// ===========================================================================
// sq-rf9uv — `Policy::party_collections`: collection IDENTITY retained from the policy
// DOCUMENT, carried independently of any member list. A consumer that freezes a rule
// into an identity-matched head (the sparq-solid ODRL bridge) must recognise a
// collection with ZERO supplied membership edges, which request evidence alone can
// never do. [SONNET-4.6]
// ===========================================================================

#[test]
fn party_collections_retains_declared_collection_identity() {
    let p = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<http://example.org/team>  a odrl:PartyCollection .
<http://example.org/alice> odrl:partOf <http://example.org/crew> .
<http://example.org/photos> a odrl:AssetCollection .
<http://example.org/img1>  odrl:partOf <http://example.org/photos> .
<urn:pol/pc> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:read ; odrl:target <http://example.org/x> ;
    odrl:assignee <http://example.org/team> ] .
"#,
        "turtle",
    )
    .expect("policy parses");

    // The explicit type declaration — retained with NO membership edge anywhere.
    assert!(
        p.party_collections.contains("http://example.org/team"),
        "an `a odrl:PartyCollection` subject is a collection even with zero members: {:?}",
        p.party_collections
    );
    // The object of a policy-stated `odrl:partOf` edge — the same identity, expressed
    // the other common way round.
    assert!(
        p.party_collections.contains("http://example.org/crew"),
        "the object of an `odrl:partOf` edge is a collection: {:?}",
        p.party_collections
    );
    // An explicitly-typed AssetCollection is NOT admitted through the shared
    // `odrl:partOf` predicate.
    assert!(
        !p.party_collections.contains("http://example.org/photos"),
        "an `a odrl:AssetCollection` is not a party collection: {:?}",
        p.party_collections
    );
    // Nothing else is: neither the member, nor the rule target, nor the policy IRI.
    assert!(!p.party_collections.contains("http://example.org/alice"), "a member is not a collection");
    assert!(!p.party_collections.contains("http://example.org/x"), "the rule target is not a collection");
    assert!(!p.party_collections.contains("urn:pol/pc"), "the policy is not a collection");
}

#[test]
fn party_collections_is_empty_when_the_document_declares_none() {
    let p = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/plain> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <http://example.org/x> ;
    odrl:assignee <http://example.org/alice> ] .
"#,
        "turtle",
    )
    .expect("policy parses");
    assert!(
        p.party_collections.is_empty(),
        "a plain assignee IRI is never inferred to be a collection: {:?}",
        p.party_collections
    );
}
