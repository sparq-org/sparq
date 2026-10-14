//! End-to-end tests for the structure-aware-vectorisation **P2** surface (sq-0wo9e.3): the SHACL/OWL
//! prior extractor + the QUDT unit-normaliser, exercised through the REAL cross-module path — a
//! parsed `sparq-shacl` shapes model feeds the codebook + cardinality priors, and unit-normalised
//! magnitudes feed the P1 order-preserving numeric encoder.
#![cfg(feature = "structure-shacl")]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_vectors::{
    normalise, route, Cardinality, Codebook, Encoder, NumericEncoder, QuantityKind, ShaclPriors,
    INVALID_SLOT,
};
use sparq_shacl::model::ShapesModel;

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(s))
}

const SHAPES: &str = r#"
@prefix sh:   <http://www.w3.org/ns/shacl#> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
@prefix ex:   <http://ex/> .

ex:SensorShape a sh:NodeShape ;
  sh:targetClass ex:Sensor ;
  sh:property [ sh:path ex:status ; sh:in ( ex:Active ex:Idle ex:Fault ) ; sh:maxCount 1 ] ;
  sh:property [ sh:path ex:reading ; sh:datatype xsd:decimal ] ;
  sh:property [ sh:path ex:tag ; sh:datatype xsd:string ] .
"#;

/// The whole P2 SHACL path: parse shapes → mine priors → build a codebook from the mined enum →
/// a member encodes to its slot, a NON-member to the reserved invalid code a closed-world SHACL
/// `sh:in` shape rejects. This exercises the real `ShapesModel::parse` → `ShaclPriors` → `Codebook`
/// chain, not a hand-built codebook.
#[test]
fn shacl_enum_prior_drives_slot_match_codebook() {
    let shapes_graph = Graph::load_str(SHAPES, "turtle").unwrap();
    let model = ShapesModel::parse(&shapes_graph);
    let priors = ShaclPriors::from_model(&model);

    let status = priors.get("http://ex/status").expect("status prior mined from sh:in");
    let cb: &Codebook = status.enum_codebook.as_ref().expect("sh:in → codebook");
    assert_eq!(cb.member_count(), 3);

    // A declared member encodes to its own slot (exact, not a cosine threshold).
    let active_block = cb.encode(&iri("http://ex/Active"));
    assert_eq!(cb.decode(&active_block).as_ref(), Some(&iri("http://ex/Active")));

    // A value NOT in the enum lands on the reserved invalid slot — the code a SHACL `sh:in` shape
    // rejects (out-of-enum, never silently a member). This is the design's answer-safety property.
    let outsider = iri("http://ex/Booting");
    assert!(!cb.is_member(&outsider));
    let bad = cb.encode(&outsider);
    assert_eq!(bad[INVALID_SLOT], 1.0);
    assert!(cb.decode(&bad).is_none());
}

/// The cardinality pooling rule + datatype-confirm flow through the real shapes model: `sh:maxCount
/// 1` → Functional (one slot); a declared `sh:datatype` confirms the router-chosen encoder lane.
#[test]
fn shacl_cardinality_and_datatype_confirm() {
    let shapes_graph = Graph::load_str(SHAPES, "turtle").unwrap();
    let model = ShapesModel::parse(&shapes_graph);
    let priors = ShaclPriors::from_model(&model);

    // status: maxCount 1 → functional (single deterministic slot).
    assert_eq!(priors.get("http://ex/status").unwrap().cardinality, Cardinality::Functional);
    // reading: xsd:decimal, no maxCount → Multi, and the datatype confirms the numeric lane.
    let reading = priors.get("http://ex/reading").unwrap();
    assert_eq!(reading.cardinality, Cardinality::Multi);
    assert_eq!(reading.datatype_encoder, Some(Encoder::Numeric));
    // The declared datatype agrees with what the standalone router would pick (router-confirm).
    assert_eq!(
        reading.datatype_encoder,
        Some(route("http://www.w3.org/2001/XMLSchema#decimal"))
    );
    // tag: xsd:string → the text (Other) lane.
    assert_eq!(priors.get("http://ex/tag").unwrap().datatype_encoder, Some(Encoder::Other));
}

/// The load-bearing P2 numeric invariant: unit-normalise BEFORE the order-preserving numeric encoder
/// so two literals denoting the SAME physical length encode to the IDENTICAL code. `1000 m` and
/// `1 km` must produce byte-identical P1 numeric blocks; a genuinely different length (`2 km`) must
/// not. This is the real `units::normalise` → `encode::NumericEncoder` chain.
#[test]
fn unit_normalised_magnitudes_share_a_code() {
    // Observed canonical (metre) values for the predicate — fit the P1 numeric encoder over them.
    let observed_m = [
        normalise(500.0, "M").unwrap().canonical_value,    // 500 m
        normalise(1000.0, "M").unwrap().canonical_value,   // 1000 m == 1 km
        normalise(2.0, "KiloM").unwrap().canonical_value,  // 2 km == 2000 m
    ];
    for n in &observed_m {
        assert!(n.is_finite());
    }
    let enc = NumericEncoder::fit(observed_m, 16);

    // 1000 m and 1 km are the same length → identical canonical value → identical encoded block.
    let m1000 = normalise(1000.0, "M").unwrap();
    let km1 = normalise(1.0, "KiloM").unwrap();
    assert_eq!(m1000.kind, QuantityKind::Length);
    assert_eq!(km1.kind, QuantityKind::Length);
    assert_eq!(m1000.canonical_value, km1.canonical_value, "1000 m == 1 km canonically");
    assert_eq!(
        enc.encode(m1000.canonical_value),
        enc.encode(km1.canonical_value),
        "same physical length must encode to the identical numeric block"
    );

    // A different length (2 km == 2000 m) must encode DIFFERENTLY — normalisation did not flatten
    // genuine differences, only spurious unit ones.
    let km2 = normalise(2.0, "KiloM").unwrap();
    assert_ne!(
        enc.encode(m1000.canonical_value),
        enc.encode(km2.canonical_value),
        "2 km is a genuinely different length and must encode differently"
    );

    // A mass annotated where a length is expected is a SHACL-detectable mismatch: its kind differs,
    // so a caller must not feed it into THIS (length) numeric block.
    let mass = normalise(1000.0, "KiloGM").unwrap();
    assert_ne!(mass.kind, m1000.kind, "a mass must not silently share the length block");
}
