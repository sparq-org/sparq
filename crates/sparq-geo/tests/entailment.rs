//! sq-5ts8 — GeoSPARQL RDFS/OWL Feature/Geometry class entailment
//! conformance, plus an honest gap-marker for the query-rewrite extension.
//!
//! The OGC GeoSPARQL compliance ratchet (`tests/ogc_compliance_ratchet.rs`,
//! sq-9h1r) exercises ONLY the *topology vocabulary extension* — the
//! `geof:sf*` / `geof:eh*` / `geof:rcc8*` FILTER functions. Two further OGC
//! sub-conformance classes were tracked-but-unexercised; this file closes the
//! one whose engine support exists today and DOCUMENTS the one whose support
//! does not.
//!
//! # 1. RDFS/OWL entailment requirements (EXERCISED here)
//!
//! GeoSPARQL 1.0 Req 4-7 / 1.1 §6 require that a conforming "RDFS/OWL
//! entailment" implementation derives the standard consequences of the
//! GeoSPARQL ontology axioms — the `geo:Feature` / `geo:Geometry` /
//! `geo:SpatialObject` class hierarchy and the `geo:hasGeometry` /
//! `geo:hasDefaultGeometry` property axioms. sparq has NO geo-specific
//! reasoner: this requirement is met by the GENERIC `sparq-reason` RDFS /
//! OWL-RL rule set (rdfs2/3/7/9/11 etc.) applied to those ontology axioms.
//! These fixtures load the relevant slice of the GeoSPARQL ontology (TBox) plus
//! a tiny instance graph (ABox) and assert the closure contains exactly the
//! triples the OGC entailment requirements demand — and, negatively, that the
//! reasoner does not over-entail.
//!
//! The axioms encoded are the entailment-relevant declarations from the OGC
//! GeoSPARQL ontology (<http://www.opengis.net/ont/geosparql>):
//!
//! ```text
//! geo:Feature            rdfs:subClassOf    geo:SpatialObject .
//! geo:Geometry           rdfs:subClassOf    geo:SpatialObject .
//! geo:hasGeometry        rdfs:domain        geo:Feature .
//! geo:hasGeometry        rdfs:range         geo:Geometry .
//! geo:hasDefaultGeometry rdfs:subPropertyOf geo:hasGeometry .
//! geo:defaultGeometry    rdfs:subPropertyOf geo:hasDefaultGeometry .  # 1.1
//! ```
//!
//! # 2. Query-rewrite extension (implemented — see `tests/query_rewrite.rs`)
//!
//! The GeoSPARQL *query-rewrite extension* lets a TRIPLE PATTERN with a topology
//! property predicate — `?a geo:sfWithin ?b` — be answered as if the
//! corresponding `geof:sfWithin(?a,?b)` FILTER had been written. As of sq-9g58
//! sparq implements this as a SPARQL ALGEBRA rewrite on the dedicated
//! [`sparq_geo::geosparql_rewrite`] entry point (end-to-end fixtures live in
//! `tests/query_rewrite.rs`). It is NOT a materialization rule: the property form
//! is not derived into the graph by the RDFS/OWL closure — it is expanded at
//! query time into geometry-resolution joins + a `geof:` FILTER. The test below,
//! [`query_rewrite_property_form_is_not_materialized`], pins exactly that boundary:
//! the *reasoner* still does not (and must not) manufacture a `geo:sfWithin`
//! triple — that is the query-rewrite path's job, on its own entry point, leaving
//! the standard entry points (and RDFS materialization) W3C-conformant.

use oxrdf::vocab::{rdf, rdfs};
use oxrdf::{NamedNode, Term};
use sparq_core::dict::{Dict, Id};
use sparq_geo::vocab::{HAS_DEFAULT_GEOMETRY, HAS_GEOMETRY};
use sparq_reason::{materialize_owl_rl, materialize_rdfs};

// ---- GeoSPARQL ontology IRIs touched by the entailment axioms --------------
const GEO_FEATURE: &str = "http://www.opengis.net/ont/geosparql#Feature";
const GEO_GEOMETRY: &str = "http://www.opengis.net/ont/geosparql#Geometry";
const GEO_SPATIAL_OBJECT: &str = "http://www.opengis.net/ont/geosparql#SpatialObject";
const GEO_DEFAULT_GEOMETRY: &str = "http://www.opengis.net/ont/geosparql#defaultGeometry";

/// Intern an IRI to its dict id.
fn iri(dict: &mut Dict, s: &str) -> Id {
    dict.intern_iri(s)
}

/// Look up an already-interned triple by IRI strings; `None` if any term is
/// unknown to the dict (id 0), which itself proves the triple is in no id-set.
fn lookup3(dict: &Dict, s: &str, p: &str, o: &str) -> Option<[Id; 3]> {
    let g = |i: &str| {
        let id = dict.lookup(&Term::NamedNode(NamedNode::new_unchecked(i.to_string())));
        (id != 0).then_some(id)
    };
    Some([g(s)?, g(p)?, g(o)?])
}

/// Assert `(s,p,o)` is PRESENT in the closure (an OGC entailment consequence).
fn assert_entailed(dict: &Dict, set: &[[Id; 3]], s: &str, p: &str, o: &str) {
    let want = lookup3(dict, s, p, o)
        .unwrap_or_else(|| panic!("expected-present triple has an unknown term: ({s} {p} {o})"));
    assert!(
        set.contains(&want),
        "GeoSPARQL entailment requirement unmet: expected ({s} {p} {o})"
    );
}

/// Assert `(s,p,o)` is ABSENT from the closure (guards against over-entailment).
fn assert_not_entailed(dict: &Dict, set: &[[Id; 3]], s: &str, p: &str, o: &str) {
    if let Some(t) = lookup3(dict, s, p, o) {
        assert!(
            !set.contains(&t),
            "reasoner OVER-ENTAILED GeoSPARQL: ({s} {p} {o}) must be absent"
        );
    }
}

/// The GeoSPARQL ontology entailment-relevant axioms (TBox) plus a tiny
/// instance graph (ABox), as interned id-triples ready for materialization.
///
/// ABox:
/// * `ex:london  geo:hasGeometry        ex:londonGeom`  — an explicit geometry.
/// * `ex:paris   geo:hasDefaultGeometry ex:parisGeom`   — a DEFAULT geometry.
/// * `ex:berlin  geo:defaultGeometry    ex:berlinGeom`  — the 1.1 spelling.
fn geosparql_fixture(dict: &mut Dict) -> Vec<[Id; 3]> {
    let ty = iri(dict, rdf::TYPE.as_str());
    let sub_class = iri(dict, rdfs::SUB_CLASS_OF.as_str());
    let sub_prop = iri(dict, rdfs::SUB_PROPERTY_OF.as_str());
    let domain = iri(dict, rdfs::DOMAIN.as_str());
    let range = iri(dict, rdfs::RANGE.as_str());

    let feature = iri(dict, GEO_FEATURE);
    let geometry = iri(dict, GEO_GEOMETRY);
    let spatial_object = iri(dict, GEO_SPATIAL_OBJECT);
    let has_geom = iri(dict, HAS_GEOMETRY);
    let has_default_geom = iri(dict, HAS_DEFAULT_GEOMETRY);
    let default_geom = iri(dict, GEO_DEFAULT_GEOMETRY);

    let london = iri(dict, "http://ex/london");
    let paris = iri(dict, "http://ex/paris");
    let berlin = iri(dict, "http://ex/berlin");
    let london_geom = iri(dict, "http://ex/londonGeom");
    let paris_geom = iri(dict, "http://ex/parisGeom");
    let berlin_geom = iri(dict, "http://ex/berlinGeom");

    vec![
        // ---- TBox: GeoSPARQL ontology axioms -------------------------------
        [feature, sub_class, spatial_object],
        [geometry, sub_class, spatial_object],
        [has_geom, domain, feature],
        [has_geom, range, geometry],
        [has_default_geom, sub_prop, has_geom],
        [default_geom, sub_prop, has_default_geom], // GeoSPARQL 1.1
        // ---- ABox: instances using each property ---------------------------
        [london, has_geom, london_geom],
        [paris, has_default_geom, paris_geom],
        [berlin, default_geom, berlin_geom],
        // Keep the ty id live in the dict even if no asserted (x rdf:type y) is
        // present, so lookups for entailed type triples resolve.
        [london, ty, feature],
    ]
}

/// RDFS-entailment conformance: the GeoSPARQL class/property axioms must yield
/// the standard consequences over the instance data.
#[test]
fn rdfs_entailment_over_geosparql_ontology() {
    let mut dict = Dict::new();
    let mut triples = geosparql_fixture(&mut dict);
    let added = materialize_rdfs(&mut dict, &mut triples);
    assert!(
        added > 0,
        "RDFS closure derived nothing over the GeoSPARQL ontology"
    );

    // rdfs7 (subPropertyOf): hasDefaultGeometry ⊑ hasGeometry, so a default
    // geometry IS a geometry — Paris gains an explicit geo:hasGeometry.
    assert_entailed(
        &dict,
        &triples,
        "http://ex/paris",
        HAS_GEOMETRY,
        "http://ex/parisGeom",
    );
    // GeoSPARQL 1.1: defaultGeometry ⊑ hasDefaultGeometry ⊑ hasGeometry — two
    // subPropertyOf hops (rdfs5 closes the chain), so Berlin too.
    assert_entailed(
        &dict,
        &triples,
        "http://ex/berlin",
        HAS_DEFAULT_GEOMETRY,
        "http://ex/berlinGeom",
    );
    assert_entailed(
        &dict,
        &triples,
        "http://ex/berlin",
        HAS_GEOMETRY,
        "http://ex/berlinGeom",
    );

    // rdfs2 (domain): hasGeometry domain Feature — every subject of a (default)
    // geometry property is a geo:Feature.
    assert_entailed(
        &dict,
        &triples,
        "http://ex/london",
        rdf::TYPE.as_str(),
        GEO_FEATURE,
    );
    assert_entailed(
        &dict,
        &triples,
        "http://ex/paris",
        rdf::TYPE.as_str(),
        GEO_FEATURE,
    );
    assert_entailed(
        &dict,
        &triples,
        "http://ex/berlin",
        rdf::TYPE.as_str(),
        GEO_FEATURE,
    );

    // rdfs3 (range): hasGeometry range Geometry — every object is a geo:Geometry.
    assert_entailed(
        &dict,
        &triples,
        "http://ex/londonGeom",
        rdf::TYPE.as_str(),
        GEO_GEOMETRY,
    );
    assert_entailed(
        &dict,
        &triples,
        "http://ex/parisGeom",
        rdf::TYPE.as_str(),
        GEO_GEOMETRY,
    );
    assert_entailed(
        &dict,
        &triples,
        "http://ex/berlinGeom",
        rdf::TYPE.as_str(),
        GEO_GEOMETRY,
    );

    // rdfs9 (subClassOf type): Feature ⊑ SpatialObject, Geometry ⊑ SpatialObject
    // — both a feature and its geometry are geo:SpatialObjects.
    assert_entailed(
        &dict,
        &triples,
        "http://ex/london",
        rdf::TYPE.as_str(),
        GEO_SPATIAL_OBJECT,
    );
    assert_entailed(
        &dict,
        &triples,
        "http://ex/londonGeom",
        rdf::TYPE.as_str(),
        GEO_SPATIAL_OBJECT,
    );

    // NEGATIVE: the reasoner must not collapse the hierarchy backwards — a
    // SpatialObject is NOT necessarily a Feature, and a Feature is NOT a
    // Geometry. (Anchors: the positives above already proved the regime fired.)
    assert_not_entailed(
        &dict,
        &triples,
        GEO_SPATIAL_OBJECT,
        rdfs::SUB_CLASS_OF.as_str(),
        GEO_FEATURE,
    );
    assert_not_entailed(
        &dict,
        &triples,
        "http://ex/london",
        rdf::TYPE.as_str(),
        GEO_GEOMETRY,
    );
    assert_not_entailed(
        &dict,
        &triples,
        "http://ex/londonGeom",
        rdf::TYPE.as_str(),
        GEO_FEATURE,
    );
    // hasGeometry is NOT a sub-property of hasDefaultGeometry (the axiom is the
    // other way round): London's explicit geometry is not a DEFAULT geometry.
    assert_not_entailed(
        &dict,
        &triples,
        "http://ex/london",
        HAS_DEFAULT_GEOMETRY,
        "http://ex/londonGeom",
    );
}

/// OWL-RL is a superset of RDFS, so every RDFS GeoSPARQL consequence must still
/// hold under the OWL-RL regime (the profile a GeoSPARQL "RDFS/OWL entailment"
/// deployment would actually select).
#[test]
fn owl_rl_entailment_superset_of_rdfs() {
    let mut dict = Dict::new();
    let mut triples = geosparql_fixture(&mut dict);
    let added = materialize_owl_rl(&mut dict, &mut triples);
    assert!(
        added > 0,
        "OWL-RL closure derived nothing over the GeoSPARQL ontology"
    );

    // Same OGC entailment consequences as the RDFS test (OWL-RL ⊇ RDFS).
    assert_entailed(
        &dict,
        &triples,
        "http://ex/paris",
        HAS_GEOMETRY,
        "http://ex/parisGeom",
    );
    assert_entailed(
        &dict,
        &triples,
        "http://ex/berlin",
        HAS_GEOMETRY,
        "http://ex/berlinGeom",
    );
    assert_entailed(
        &dict,
        &triples,
        "http://ex/paris",
        rdf::TYPE.as_str(),
        GEO_FEATURE,
    );
    assert_entailed(
        &dict,
        &triples,
        "http://ex/parisGeom",
        rdf::TYPE.as_str(),
        GEO_GEOMETRY,
    );
    assert_entailed(
        &dict,
        &triples,
        "http://ex/london",
        rdf::TYPE.as_str(),
        GEO_SPATIAL_OBJECT,
    );
    assert_entailed(
        &dict,
        &triples,
        "http://ex/londonGeom",
        rdf::TYPE.as_str(),
        GEO_SPATIAL_OBJECT,
    );
}

/// Materialization is idempotent: re-running over an already-closed set adds
/// nothing (mirrors the reasoner's documented idempotence — a GeoSPARQL store
/// re-materialized after a no-op edit must not grow).
#[test]
fn entailment_is_idempotent() {
    let mut dict = Dict::new();
    let mut triples = geosparql_fixture(&mut dict);
    materialize_rdfs(&mut dict, &mut triples);
    let second = materialize_rdfs(&mut dict, &mut triples);
    assert_eq!(
        second, 0,
        "second RDFS materialization over a closed GeoSPARQL set added triples"
    );
}

/// The query-rewrite extension is a QUERY-TIME transform, NOT a materialization
/// rule: the RDFS/OWL reasoner must not manufacture a `geo:sfWithin` triple.
///
/// The property form IS supported — via [`sparq_geo::geosparql_rewrite`], with
/// end-to-end fixtures in `tests/query_rewrite.rs`. This test pins the
/// complementary boundary: even though the geometries genuinely stand in the
/// `sfWithin` relation, the RDFS closure does NOT derive the topology property
/// (no rule manufactures it), so a `geo:sfWithin` predicate matched on the
/// STANDARD (non-rewrite) entry points binds only asserted triples — there are
/// none here. The rewrite path (separate entry point) is what answers the
/// property form; the reasoner stays W3C-conformant. sq-9g58
#[test]
fn query_rewrite_property_form_is_not_materialized() {
    const GEO_SF_WITHIN: &str = "http://www.opengis.net/ont/geosparql#sfWithin";

    let mut dict = Dict::new();
    // The geometries genuinely stand in the sfWithin relation (a point inside a
    // square), but the relation is expressed only via geometry serialisations,
    // not as an asserted geo:sfWithin triple.
    let small = iri(&mut dict, "http://ex/small");
    let big = iri(&mut dict, "http://ex/big");
    let as_wkt = iri(&mut dict, sparq_geo::vocab::AS_WKT);
    // WKT literals as opaque dict terms (the reasoner does not interpret them).
    let small_wkt = dict.intern(&Term::NamedNode(NamedNode::new_unchecked(
        "http://ex/POINT_1_1".to_string(),
    )));
    let big_wkt = dict.intern(&Term::NamedNode(NamedNode::new_unchecked(
        "http://ex/SQUARE".to_string(),
    )));
    // Pre-intern the sfWithin predicate so the lookup can resolve a real id —
    // proving the triple's ABSENCE is genuine set non-membership, not an
    // unknown-term artefact.
    let _ = iri(&mut dict, GEO_SF_WITHIN);

    let mut triples = vec![[small, as_wkt, small_wkt], [big, as_wkt, big_wkt]];
    materialize_rdfs(&mut dict, &mut triples);

    // The property form is NOT MATERIALIZED: no `ex:small geo:sfWithin ex:big`
    // triple is derived into the graph by the RDFS closure. (The relation IS
    // queryable via the query-rewrite extension — `sparq_geo::geosparql_rewrite`,
    // tested in `tests/query_rewrite.rs` — which expands the property form at
    // query time rather than materializing it.)
    assert_not_entailed(
        &dict,
        &triples,
        "http://ex/small",
        GEO_SF_WITHIN,
        "http://ex/big",
    );

    // Sanity: the supported surface is the lexical `geof:sfWithin` FUNCTION,
    // which DOES report the relation. This is the contrast — function: yes,
    // property-pattern rewrite: not yet.
    let got = sparq_geo::geof::lex::sf_within("POINT(1 1)", "POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))")
        .expect("geof:sfWithin lexical evaluates");
    assert!(
        got,
        "geof:sfWithin FUNCTION must hold for a point inside the square"
    );
}
