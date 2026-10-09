//! [OPUS-4.8] sq-wukl — the `odrl:spatial` dimension + region `isPartOf` trees.
//!
//! This is the SPATIAL twin of the DPV/purpose-taxonomy subsumption that landed in
//! sq-z3ve (#503, `tests/odrl_eval.rs`). A `spatial isPartOf <Region>` constraint is
//! satisfied when the request's stated region IS the named region OR is **transitively
//! part-of** it under the region `isPartOf` tree the request supplies as subsumption
//! evidence — the SAME caller-supplied closure the purpose taxonomy uses
//! ([`Request::with_purpose_subsumption`] / [`Request::with_purpose_taxonomy`]). There
//! is no separate spatial evidence channel: spatial reuses the merged subsumption code
//! path, so all of the soundness/fail-closed properties of sq-z3ve carry over verbatim.
//!
//! The honesty contract (mirrors purpose/recipient/dateTime, sq-q56r/sq-5037/sq-idnv):
//! the tree is **evidence the request supplies**, never invented. No tree supplied ⇒
//! exact membership (fully backward compatible), and matching never widens access on an
//! edge the request did not assert — a missing edge is fail-closed.

use sparq_policy::{
    evaluate, parse_policy_str, prohibition_status, spatial_status, ProhibitionStatus, Request,
    SpatialMatch, Value, ODRL_SPATIAL,
};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

fn left(local: &str) -> String {
    format!("{ODRL}{local}")
}

// ---------------------------------------------------------------------------
// Spatial hierarchy: a permission gated on a region grants a request from a
// sub-region declared isPartOf it (the spatial dual of purpose subsumption).
// ---------------------------------------------------------------------------
const EU: &str = "http://publications.europa.eu/resource/authority/country/EU";
const DE: &str = "http://publications.europa.eu/resource/authority/country/DEU";
const BERLIN: &str = "https://sws.geonames.org/2950159/"; // Berlin
const USA: &str = "http://publications.europa.eu/resource/authority/country/USA";

/// "MAY distribute asset-X to recipients in the EU region" — spatial isPartOf EU.
fn eu_region_permission() -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/geo> a odrl:Set ; odrl:permission [
    odrl:action odrl:distribute ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:spatial ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand <{EU}> ] ] .
"#
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

#[test]
fn spatial_subregion_is_part_of_region_grants() {
    let p = eu_region_permission();
    let base = Request::new(left("distribute")).on("urn:asset/x");

    // Germany isPartOf EU → a sub-region under the named region → ALLOW.
    let germany = base
        .clone()
        .with(ODRL_SPATIAL, Value::Iri(DE.into()))
        .with_purpose_subsumption(DE, EU);
    assert!(
        evaluate(&p, &germany).allow,
        "a sub-region declared part-of the named region grants"
    );
    assert_eq!(
        spatial_status(&p.permissions[0], &germany),
        SpatialMatch::Satisfied
    );
}

#[test]
fn spatial_transitive_city_grants() {
    let p = eu_region_permission();
    let base = Request::new(left("distribute")).on("urn:asset/x");

    // Berlin isPartOf Germany isPartOf EU — transitive region tree (two hops).
    let berlin = base
        .clone()
        .with(ODRL_SPATIAL, Value::Iri(BERLIN.into()))
        .with_purpose_subsumption(BERLIN, DE)
        .with_purpose_subsumption(DE, EU);
    assert!(
        evaluate(&p, &berlin).allow,
        "transitive spatial isPartOf (city in country in region) grants"
    );
    assert_eq!(
        spatial_status(&p.permissions[0], &berlin),
        SpatialMatch::Satisfied
    );
}

#[test]
fn spatial_bulk_taxonomy_builder_grants() {
    // The bulk `with_purpose_taxonomy` builder also feeds the spatial dimension —
    // order-independent closure over a whole region tree at once.
    let p = eu_region_permission();
    let req = Request::new(left("distribute"))
        .on("urn:asset/x")
        .with(ODRL_SPATIAL, Value::Iri(BERLIN.into()))
        .with_purpose_taxonomy([(DE, EU), (BERLIN, DE)]);
    assert!(
        evaluate(&p, &req).allow,
        "a bulk-supplied region tree drives the transitive spatial match"
    );
}

#[test]
fn spatial_outside_region_denied() {
    let p = eu_region_permission();
    let base = Request::new(left("distribute")).on("urn:asset/x");

    // A region not under EU (no edge reaches it) → definite no.
    let outside = base
        .clone()
        .with(ODRL_SPATIAL, Value::Iri(USA.into()))
        .with_purpose_subsumption(DE, EU); // an edge, but not for the USA
    assert!(
        !evaluate(&p, &outside).allow,
        "a region outside the named one is denied"
    );
    assert_eq!(
        spatial_status(&p.permissions[0], &outside),
        SpatialMatch::DefinitelyUnsatisfied
    );
}

#[test]
fn spatial_hierarchy_without_edge_fails_closed() {
    // THE honesty test: a sub-region is NOT subsumed unless the request actually
    // supplies the isPartOf edge. No invented subsumption — a missing edge is
    // fail-closed (and the stated region is then provably != the named one → denied).
    let p = eu_region_permission();
    let no_edge = Request::new(left("distribute"))
        .on("urn:asset/x")
        .with(ODRL_SPATIAL, Value::Iri(DE.into())); // Germany, but NO isPartOf edge
    assert!(
        !evaluate(&p, &no_edge).allow,
        "without a supplied isPartOf edge a sub-region does NOT grant (no invented subsumption)"
    );
    assert_eq!(
        spatial_status(&p.permissions[0], &no_edge),
        SpatialMatch::DefinitelyUnsatisfied
    );
}

#[test]
fn spatial_exact_region_still_matches_with_or_without_tree() {
    // Backward compatibility: the exact named region matches, tree or no tree.
    let p = eu_region_permission();
    let base = Request::new(left("distribute")).on("urn:asset/x");

    let exact = base.clone().with(ODRL_SPATIAL, Value::Iri(EU.into()));
    assert!(evaluate(&p, &exact).allow, "the exact named region matches");

    let exact_with_unrelated_tree = base
        .with(ODRL_SPATIAL, Value::Iri(EU.into()))
        .with_purpose_subsumption(BERLIN, DE);
    assert!(
        evaluate(&p, &exact_with_unrelated_tree).allow,
        "an unrelated tree does not break the exact match"
    );
}

#[test]
fn spatial_missing_evidence_unprovable() {
    let p = eu_region_permission();
    // No spatial evidence at all → Unprovable → fail-closed.
    let none = Request::new(left("distribute")).on("urn:asset/x");
    assert!(
        !evaluate(&p, &none).allow,
        "missing spatial evidence fails closed"
    );
    assert_eq!(
        spatial_status(&p.permissions[0], &none),
        SpatialMatch::Unprovable
    );
}

#[test]
fn spatial_not_constrained_when_absent() {
    let ttl = format!(
        r#"@prefix odrl: <{ODRL}> . <urn:pol/ns> a odrl:Set ; odrl:permission [
           odrl:action odrl:distribute ; odrl:target <urn:asset/x> ] ."#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let req = Request::new(left("distribute"))
        .on("urn:asset/x")
        .with(ODRL_SPATIAL, Value::Iri(DE.into()));
    assert_eq!(
        spatial_status(&p.permissions[0], &req),
        SpatialMatch::NotConstrained
    );
}

// ---------------------------------------------------------------------------
// Prohibition dual: a carve-out gated on a spatial region carves out a sub-region
// declared part-of it; an unrelated region is Withdrawn; missing evidence Ambiguous.
// ---------------------------------------------------------------------------
#[test]
fn spatial_prohibition_dual() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/pgeo> a odrl:Set ; odrl:prohibition [
    odrl:action odrl:distribute ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:spatial ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand <{EU}> ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let base = Request::new(left("distribute")).on("urn:asset/x");

    // Germany part-of EU → the carve-out applies (a sub-region IS in the region).
    let germany = base
        .clone()
        .with(ODRL_SPATIAL, Value::Iri(DE.into()))
        .with_purpose_subsumption(DE, EU);
    assert_eq!(prohibition_status(&p, &germany), ProhibitionStatus::Applies);

    // A region outside EU → definitely not carved out → Withdrawn.
    let outside = base
        .clone()
        .with(ODRL_SPATIAL, Value::Iri(USA.into()))
        .with_purpose_subsumption(DE, EU);
    assert_eq!(
        prohibition_status(&p, &outside),
        ProhibitionStatus::Withdrawn
    );

    // No spatial evidence → unprovable → Ambiguous (deny kept, fail-closed).
    assert_eq!(prohibition_status(&p, &base), ProhibitionStatus::Ambiguous);
}

// ---------------------------------------------------------------------------
// The region tree applies to an explicit isPartOf SET right-operand too.
// ---------------------------------------------------------------------------
#[test]
fn spatial_is_part_of_set_with_hierarchy() {
    // A constraint naming a SET {EU|USA}; a request stating a narrower region under EU
    // (via an edge) matches the EU member of the set.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/set> a odrl:Set ; odrl:permission [
    odrl:action odrl:distribute ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:spatial ; odrl:operator odrl:isPartOf ;
                      odrl:rightOperand "{EU}|{USA}" ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let germany = Request::new(left("distribute"))
        .on("urn:asset/x")
        .with(ODRL_SPATIAL, Value::Str(DE.into()))
        .with_purpose_subsumption(DE, EU);
    assert!(
        evaluate(&p, &germany).allow,
        "a sub-region under one named set member matches"
    );
}

// ---------------------------------------------------------------------------
// Cycle-safety: a malformed region tree with a cycle must terminate (and not match
// a value outside the named region). The closure maintenance in sq-z3ve already
// tolerates cycles; this pins it at the public spatial-API boundary.
// ---------------------------------------------------------------------------
#[test]
fn spatial_cycle_terminates_and_is_fail_closed() {
    let p = eu_region_permission(); // gated on EU
    let base = Request::new(left("distribute")).on("urn:asset/x");

    // A → B → A is a cycle that does NOT reach EU. Must terminate, not match.
    let cyclic = base
        .clone()
        .with(ODRL_SPATIAL, Value::Iri("urn:region/A".into()))
        .with_purpose_subsumption("urn:region/A", "urn:region/B")
        .with_purpose_subsumption("urn:region/B", "urn:region/A");
    assert!(
        !evaluate(&p, &cyclic).allow,
        "a cyclic region tree not reaching the named region must terminate and deny"
    );
}
