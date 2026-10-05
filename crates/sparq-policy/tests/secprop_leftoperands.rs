//! Integration suite for the sparq security-property ODRL profile — the
//! `secx:requires…` leftOperands made first-class (`secprop-leftoperands` feature).
//!
//! Exercises the REAL `evaluate` path with a `secx:requires…` constraint (the
//! load-bearing invariant: making the leftOperands first-class does NOT change the
//! stateless evaluator — a `secx:` constraint still evaluates as an opaque
//! custom-leftOperand block, fail-closed) plus the profile-recognition surface.
//!
//! [OPUS-4.8] sq-uor3g (epic sq-0dksu, Phase 4). 🤖 SPARQ agent —
//! security-properties ontology. Flag for re-review when Fable returns.
#![cfg(feature = "secprop-leftoperands")]

use sparq_policy::secprop::{
    discharge_requirements, is_secprop_left_operand, over_dimension, Deontic, DischargeExpr,
    PROFILE_IRI, REQUIRES_ASSURANCE, REQUIRES_UNLINKABILITY_SCOPE, SECPROP_LEFT_OPERANDS,
};
use sparq_policy::{
    evaluate, parse_policy_str, Action, Constraint, Operator, Policy, Request, Rule, Value, ODRL_NS,
};

/// The recogniser + dimension map are wired through the public crate surface.
#[test]
fn profile_surface_is_first_class() {
    assert!(is_secprop_left_operand(REQUIRES_UNLINKABILITY_SCOPE));
    assert_eq!(
        over_dimension(REQUIRES_UNLINKABILITY_SCOPE),
        Some("https://w3id.org/zkp-sparql/sec-prop#UnlinkabilityScope")
    );
    assert_eq!(
        over_dimension(REQUIRES_ASSURANCE),
        Some("https://w3id.org/zkp-sparql/sec-prop#AssuranceLevel")
    );
    // Every leftOperand resolves to a distinct dimension.
    assert!(SECPROP_LEFT_OPERANDS.len() >= 7);
    assert!(PROFILE_IRI.starts_with("https://sparq.dev/ns/odrl-secprop-profile#"));
}

/// The published profile TTL parses with the real ODRL parser AND its `secx:requires…`
/// leftOperand declarations are recognised — i.e. the profile is a real, parseable RDF
/// document (not just a string blob). We parse it as an ODRL policy graph (the profile
/// triples are inert to the policy parser — no permission/prohibition rules — so the
/// parse yields an empty policy, which is the correct "the profile is data, not a
/// policy" outcome) and separately assert the leftOperand IRIs the parse must see.
#[test]
fn published_profile_ttl_is_well_formed_rdf() {
    let ttl = include_str!("../ontologies/odrl-secprop-profile.ttl");
    // The profile is declarative vocabulary, not a deontic policy: parsing it must NOT
    // error and must NOT manufacture spurious permissions/prohibitions.
    let policy = parse_policy_str(ttl, "turtle").expect("the profile TTL must be well-formed RDF");
    assert!(
        policy.permissions.is_empty() && policy.prohibitions.is_empty(),
        "the profile is vocabulary, not a policy — it must yield no rules",
    );
    // And every Rust leftOperand IRI is literally present in the published document.
    for (lo, dim) in SECPROP_LEFT_OPERANDS {
        let lo_local = lo.rsplit('#').next().unwrap();
        let dim_local = dim.rsplit('#').next().unwrap();
        assert!(
            ttl.contains(lo_local) && ttl.contains(dim_local),
            "profile TTL must declare {} over {}",
            lo,
            dim,
        );
    }
}

/// LOAD-BEARING: making the `secx:requires…` leftOperands first-class does NOT change
/// the stateless `evaluate` path. A permission gated on a `secx:requires…` constraint
/// the request supplies NO evidence for is fail-closed DENY (a custom-leftOperand
/// constraint over an unsupplied dimension is unprovable) — exactly as before this
/// feature existed. This is the answer-safety invariant.
#[test]
fn secprop_constraint_evaluates_fail_closed_unchanged() {
    let asset = "urn:asset:medical-graph";
    let party = "https://alice.example/profile#me";
    let rule = Rule {
        id: "urn:rule:1".into(),
        action: Action(format!("{ODRL_NS}read")),
        target: Some(asset.into()),
        assignee: Some(party.into()),
        assigner: None,
        // Constraint over a secx: leftOperand: "requiresUnlinkabilityScope eq CrossPresentation".
        constraints: vec![Constraint {
            left: REQUIRES_UNLINKABILITY_SCOPE.into(),
            operator: Operator::Eq,
            right: Value::Iri("https://w3id.org/zkp-sparql/sec-prop#CrossPresentation".into()),
        }],
        logical_constraints: vec![],
        duties: vec![],
    };
    let policy = Policy {
        iri: Some("urn:policy:1".into()),
        permissions: vec![rule],
        prohibitions: vec![],
        conflict: None,
        ..Policy::default()
    };

    // Request supplies NO evidence for the secx: dimension → fail-closed DENY.
    let req_no_evidence = Request::new(format!("{ODRL_NS}read")).on(asset).by(party);
    assert!(
        !evaluate(&policy, &req_no_evidence).allow,
        "a secx: constraint with no supplied evidence must fail closed (unchanged eval)",
    );

    // Request that DOES supply the matching value in context → the custom-leftOperand
    // constraint is satisfied by exact equality, so the permission grants. This proves
    // the leftOperand evaluates through the SAME generic custom-leftOperand path as
    // before — first-classing it added recognition metadata, not new eval semantics.
    let req_with_evidence = Request::new(format!("{ODRL_NS}read"))
        .on(asset)
        .by(party)
        .with(
            REQUIRES_UNLINKABILITY_SCOPE,
            Value::Iri("https://w3id.org/zkp-sparql/sec-prop#CrossPresentation".into()),
        );
    assert!(
        evaluate(&policy, &req_with_evidence).allow,
        "a satisfied secx: constraint evaluates through the unchanged custom-leftOperand path",
    );
}

/// A worked privacy preference from the design (§4.3.1) parses through the real ODRL
/// parser and the parsed `secx:requires…` leftOperands are recognised by the profile —
/// the end-to-end "a user privacy preference is an ODRL policy" path.
#[test]
fn worked_preference_parses_and_left_operands_are_recognised() {
    let ttl = r#"
@prefix odrl:     <http://www.w3.org/ns/odrl/2/> .
@prefix secx:     <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix sparqorl: <https://sparq.dev/ns/odrl-secprop-profile#> .

<urn:pref:alice-privacy> a odrl:Policy ;
  odrl:profile sparqorl: ;
  odrl:permission [
    odrl:action odrl:read ;
    odrl:constraint
      [ odrl:leftOperand  secx:requiresUnlinkabilityScope ;
        odrl:operator      odrl:gteq ;
        odrl:rightOperand  secx:CrossPresentation ] ,
      [ odrl:leftOperand  secx:requiresAssurance ;
        odrl:operator      odrl:gteq ;
        odrl:rightOperand  secx:Proven ] ] .
"#;
    let policy = parse_policy_str(ttl, "turtle").expect("worked preference must parse");
    assert_eq!(policy.permissions.len(), 1, "one permission");
    let rule = &policy.permissions[0];
    assert_eq!(rule.constraints.len(), 2, "two secx: constraints");
    // Every constraint's leftOperand is a recognised secprop leftOperand with a
    // resolvable dimension — proving the parse really produced the profile terms.
    for c in &rule.constraints {
        assert!(
            is_secprop_left_operand(&c.left),
            "parsed leftOperand {} is a recognised secprop leftOperand",
            c.left,
        );
        assert!(
            over_dimension(&c.left).is_some(),
            "parsed leftOperand {} maps to a dimension",
            c.left,
        );
    }
}

/// The ODRL half of the ZK↔ODRL constraint-discharge envelope (sq-yh427), end to end:
/// a user privacy preference written in Turtle parses, and `discharge_requirements`
/// reports exactly what a presented proof would have to establish — dimension,
/// operator and required level per rule, with the `odrl:` combinator structure intact.
///
/// It reports; it does not verify, order levels, or claim any method has any property.
#[test]
fn worked_preference_yields_the_proof_discharge_obligations() {
    let ttl = r#"
@prefix odrl:     <http://www.w3.org/ns/odrl/2/> .
@prefix secx:     <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix sparqorl: <https://sparq.dev/ns/odrl-secprop-profile#> .

<urn:pref:alice-privacy> a odrl:Policy ;
  odrl:profile sparqorl: ;
  odrl:permission [
    odrl:action odrl:read ;
    odrl:constraint
      [ odrl:leftOperand  secx:requiresUnlinkabilityScope ;
        odrl:operator      odrl:gteq ;
        odrl:rightOperand  secx:CrossPresentation ] ,
      [ odrl:leftOperand  secx:requiresAssurance ;
        odrl:operator      odrl:gteq ;
        odrl:rightOperand  secx:Proven ] ] .
"#;
    let policy = parse_policy_str(ttl, "turtle").expect("worked preference must parse");

    let requirements = discharge_requirements(&policy);
    assert_eq!(requirements.len(), 1, "one rule asks for security properties");
    assert_eq!(requirements[0].deontic, Deontic::Permission);
    // A rule's own constraints are conjoined — BOTH must hold, and the tree says so.
    let DischargeExpr::All(ref conjuncts) = requirements[0].requirement else {
        panic!("expected a conjunction, got {:?}", requirements[0].requirement);
    };
    assert_eq!(conjuncts.len(), 2);

    let obligations = requirements[0].requirement.obligations();
    assert_eq!(obligations.len(), 2, "both secx: constraints are obligations");
    for o in &obligations {
        assert_eq!(o.deontic, Deontic::Permission);
        assert_eq!(o.operator, Operator::Gteq);
        assert_eq!(
            Some(o.dimension),
            over_dimension(&o.left_operand),
            "each obligation resolves to its leftOperand's declared dimension",
        );
    }
    let mut dims: Vec<&str> = obligations.iter().map(|o| o.dimension).collect();
    dims.sort_unstable();
    assert_eq!(
        dims,
        vec![
            "https://w3id.org/zkp-sparql/sec-prop#AssuranceLevel",
            "https://w3id.org/zkp-sparql/sec-prop#UnlinkabilityScope",
        ],
    );
    assert!(
        obligations
            .iter()
            .any(|o| o.required == Value::Iri("https://w3id.org/zkp-sparql/sec-prop#Proven".into())),
        "the required level is carried through so a host can ask for it",
    );

    // LOAD-BEARING: extraction is read-only. Evidence-free evaluation of the same
    // policy is still fail-closed DENY — reporting an obligation never discharges it.
    let req = Request::new(format!("{ODRL_NS}read"));
    assert!(
        !evaluate(&policy, &req).allow,
        "extracting obligations must not change the fail-closed evaluate path",
    );
}

/// LOAD-BEARING, end to end: a preference written with `odrl:or` reads back as
/// ALTERNATIVES. A host can see from the requirement alone that establishing EITHER
/// `secx:Proven` assurance OR `secx:CrossPresentation` unlinkability suffices — it
/// never has to go back to the parsed `Policy` to learn that.
#[test]
fn an_or_preference_reads_back_as_alternatives_not_as_a_mandate() {
    let ttl = r#"
@prefix odrl:     <http://www.w3.org/ns/odrl/2/> .
@prefix secx:     <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix sparqorl: <https://sparq.dev/ns/odrl-secprop-profile#> .

<urn:pref:alice-either> a odrl:Policy ;
  odrl:profile sparqorl: ;
  odrl:permission [
    odrl:action odrl:read ;
    odrl:constraint [ odrl:or
      ( [ odrl:leftOperand  secx:requiresAssurance ;
          odrl:operator      odrl:gteq ;
          odrl:rightOperand  secx:Proven ]
        [ odrl:leftOperand  secx:requiresUnlinkabilityScope ;
          odrl:operator      odrl:gteq ;
          odrl:rightOperand  secx:CrossPresentation ] ) ] ] .
"#;
    let policy = parse_policy_str(ttl, "turtle").expect("`or` preference must parse");

    let requirements = discharge_requirements(&policy);
    assert_eq!(requirements.len(), 1);
    let DischargeExpr::All(ref conjuncts) = requirements[0].requirement else {
        panic!("expected the rule conjunction, got {:?}", requirements[0].requirement);
    };
    let DischargeExpr::Any(ref alternatives) = conjuncts[0] else {
        panic!("`odrl:or` must survive as alternatives, got {:?}", conjuncts[0]);
    };
    assert_eq!(alternatives.len(), 2, "two independent ways to discharge");
    let mut alt_dims: Vec<&str> = alternatives
        .iter()
        .map(|a| match a {
            DischargeExpr::Atomic(o) => o.dimension,
            other => panic!("expected an atomic alternative, got {:?}", other),
        })
        .collect();
    alt_dims.sort_unstable();
    assert_eq!(
        alt_dims,
        vec![
            "https://w3id.org/zkp-sparql/sec-prop#AssuranceLevel",
            "https://w3id.org/zkp-sparql/sec-prop#UnlinkabilityScope",
        ],
    );
    // The same two dimensions as the conjunctive worked preference above — which is
    // precisely why the flat inventory cannot be the requirement.
    assert_ne!(
        requirements[0].requirement,
        DischargeExpr::All(vec![DischargeExpr::All(alternatives.clone())]),
        "an `or` must not be equal to the `and` over the same leaves",
    );
}
