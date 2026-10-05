// [FABLE-5] sq-pbz04.4.1 (epic sq-pbz04.4): integration tests for the FAIL-CLOSED reverse RDF
// mapping (design record research/owl2-direct-semantics-scoping.md §L1).
//
// 🤖 SPARQ agent. Two families:
//   * ACCEPT — every ALCH class-expression / axiom / assertion shape maps to the expected
//     structural model (hand-built and asserted with `PartialEq`), plus a model→RDF→model
//     round-trip on the flat subset.
//   * REJECT — ONE test per rejection-taxonomy arm (OutOfFragment, DataConstruct, MalformedList,
//     MalformedClassExpression, Unclassifiable), plus the load-bearing fail-closed invariant:
//     a single out-of-fragment triple rejects the WHOLE graph (never a partial model).

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_dl::model::{Axiom, ClassExpression as CE, ObjectPropertyExpression as OPE};
use sparq_reason_dl::{extract, ExtractError, Ontology};

const OWL: &str = "http://www.w3.org/2002/07/owl#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

fn parse(body: &str) -> (Dict, Vec<[Id; 3]>) {
    Graph::parse_to_triples(&format!("{PRE}{body}"), "turtle").expect("parse")
}

fn iri(dict: &Dict, full: &str) -> Id {
    dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(full.to_string())))
}

/// `http://ex/{frag}` dict id.
fn ex(dict: &Dict, frag: &str) -> Id {
    iri(dict, &format!("http://ex/{frag}"))
}

fn class(dict: &Dict, frag: &str) -> CE {
    CE::Class(ex(dict, frag))
}

fn prop(dict: &Dict, frag: &str) -> OPE {
    OPE::ObjectProperty(ex(dict, frag))
}

// ============================== ACCEPT ==============================

#[test]
fn subclassof_named() {
    let (d, t) = parse(":A rdfs:subClassOf :B .");
    let o = extract(&d, &t).expect("accept");
    assert_eq!(
        o.axioms,
        vec![Axiom::SubClassOf {
            sub: class(&d, "A"),
            sup: class(&d, "B"),
        }]
    );
}

#[test]
fn equivalent_and_disjoint() {
    let (d, t) = parse(":A owl:equivalentClass :B .\n:C owl:disjointWith :D .");
    let o = extract(&d, &t).expect("accept");
    assert!(o
        .axioms
        .contains(&Axiom::EquivalentClasses(class(&d, "A"), class(&d, "B"))));
    assert!(o
        .axioms
        .contains(&Axiom::DisjointClasses(class(&d, "C"), class(&d, "D"))));
    assert_eq!(o.len(), 2);
}

#[test]
fn some_values_from() {
    let (d, t) = parse(":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ; owl:someValuesFrom :C ] .");
    let o = extract(&d, &t).expect("accept");
    assert_eq!(
        o.axioms,
        vec![Axiom::SubClassOf {
            sub: class(&d, "A"),
            sup: CE::ObjectSomeValuesFrom(prop(&d, "r"), Box::new(class(&d, "C"))),
        }]
    );
}

#[test]
fn all_values_from() {
    let (d, t) = parse(":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ; owl:allValuesFrom :C ] .");
    let o = extract(&d, &t).expect("accept");
    assert_eq!(
        o.axioms,
        vec![Axiom::SubClassOf {
            sub: class(&d, "A"),
            sup: CE::ObjectAllValuesFrom(prop(&d, "r"), Box::new(class(&d, "C"))),
        }]
    );
}

#[test]
fn intersection_of() {
    let (d, t) = parse("[ owl:intersectionOf ( :A :B ) ] rdfs:subClassOf :C .");
    let o = extract(&d, &t).expect("accept");
    assert_eq!(
        o.axioms,
        vec![Axiom::SubClassOf {
            sub: CE::ObjectIntersectionOf(vec![class(&d, "A"), class(&d, "B")]),
            sup: class(&d, "C"),
        }]
    );
}

#[test]
fn union_of() {
    let (d, t) = parse(":C rdfs:subClassOf [ owl:unionOf ( :A :B ) ] .");
    let o = extract(&d, &t).expect("accept");
    assert_eq!(
        o.axioms,
        vec![Axiom::SubClassOf {
            sub: class(&d, "C"),
            sup: CE::ObjectUnionOf(vec![class(&d, "A"), class(&d, "B")]),
        }]
    );
}

#[test]
fn complement_of() {
    let (d, t) = parse(":A rdfs:subClassOf [ owl:complementOf :B ] .");
    let o = extract(&d, &t).expect("accept");
    assert_eq!(
        o.axioms,
        vec![Axiom::SubClassOf {
            sub: class(&d, "A"),
            sup: CE::ObjectComplementOf(Box::new(class(&d, "B"))),
        }]
    );
}

#[test]
fn nested_class_expression() {
    // A ⊑ ∃r.(B ⊓ ¬C) — exercises recursion through restriction → intersection → complement.
    let (d, t) = parse(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ; owl:someValuesFrom \
         [ owl:intersectionOf ( :B [ owl:complementOf :C ] ) ] ] .",
    );
    let o = extract(&d, &t).expect("accept");
    assert_eq!(
        o.axioms,
        vec![Axiom::SubClassOf {
            sub: class(&d, "A"),
            sup: CE::ObjectSomeValuesFrom(
                prop(&d, "r"),
                Box::new(CE::ObjectIntersectionOf(vec![
                    class(&d, "B"),
                    CE::ObjectComplementOf(Box::new(class(&d, "C"))),
                ])),
            ),
        }]
    );
}

#[test]
fn sub_property_domain_range() {
    let (d, t) = parse(":r rdfs:subPropertyOf :s .\n:r rdfs:domain :A .\n:r rdfs:range :B .");
    let o = extract(&d, &t).expect("accept");
    assert!(o.axioms.contains(&Axiom::SubObjectPropertyOf {
        sub: prop(&d, "r"),
        sup: prop(&d, "s"),
    }));
    assert!(o.axioms.contains(&Axiom::ObjectPropertyDomain {
        property: prop(&d, "r"),
        domain: class(&d, "A"),
    }));
    assert!(o.axioms.contains(&Axiom::ObjectPropertyRange {
        property: prop(&d, "r"),
        range: class(&d, "B"),
    }));
    assert_eq!(o.len(), 3);
}

#[test]
fn class_assertion() {
    let (d, t) = parse(":x a :A .");
    let o = extract(&d, &t).expect("accept");
    assert_eq!(
        o.axioms,
        vec![Axiom::ClassAssertion {
            class: class(&d, "A"),
            individual: ex(&d, "x"),
        }]
    );
}

#[test]
fn object_property_assertion_declared() {
    let (d, t) = parse(":p a owl:ObjectProperty .\n:x :p :y .");
    let o = extract(&d, &t).expect("accept");
    assert_eq!(
        o.axioms,
        vec![Axiom::ObjectPropertyAssertion {
            property: prop(&d, "p"),
            source: ex(&d, "x"),
            target: ex(&d, "y"),
        }]
    );
}

#[test]
fn thing_and_nothing() {
    let (d, t) = parse(":A rdfs:subClassOf owl:Thing .\n:B rdfs:subClassOf owl:Nothing .\n:x a owl:Thing .");
    let o = extract(&d, &t).expect("accept");
    assert!(o.axioms.contains(&Axiom::SubClassOf {
        sub: class(&d, "A"),
        sup: CE::Thing,
    }));
    assert!(o.axioms.contains(&Axiom::SubClassOf {
        sub: class(&d, "B"),
        sup: CE::Nothing,
    }));
    assert!(o.axioms.contains(&Axiom::ClassAssertion {
        class: CE::Thing,
        individual: ex(&d, "x"),
    }));
}

#[test]
fn annotations_and_declarations_ignored() {
    let (d, t) = parse(
        ":A a owl:Class .\n:A rdfs:label \"a class\" .\n:A rdfs:comment \"note\" .\n\
         :O a owl:Ontology .\n:O owl:versionInfo \"1.0\" .\n:r a owl:ObjectProperty .",
    );
    let o = extract(&d, &t).expect("accept");
    assert!(o.is_empty(), "no logical axioms, got {:?}", o.axioms);
}

// ============================== ROUND-TRIP ==============================

/// Renders the FLAT axiom subset (named-class `SubClassOf`, named `ClassAssertion`, and role
/// assertions) back to RDF triples, interning an `owl:ObjectProperty` declaration for each
/// asserting property so the reverse mapping recognises it. Complex class expressions are NOT
/// round-tripped (their RDF encoding mints fresh blank nodes, so blank-node identity is not
/// preserved) — hence "where applicable".
fn render_flat(dict: &mut Dict, onto: &Ontology) -> Vec<[Id; 3]> {
    let ty = iri(dict, &format!("{RDF}type"));
    let sco = iri(dict, &format!("{RDFS}subClassOf"));
    let owl_obj = iri(dict, &format!("{OWL}ObjectProperty"));
    let mut out = Vec::new();
    let mut declared = std::collections::HashSet::new();
    for ax in &onto.axioms {
        match ax {
            Axiom::SubClassOf {
                sub: CE::Class(a),
                sup: CE::Class(b),
            } => out.push([*a, sco, *b]),
            Axiom::ClassAssertion {
                class: CE::Class(c),
                individual,
            } => out.push([*individual, ty, *c]),
            Axiom::ObjectPropertyAssertion {
                property: OPE::ObjectProperty(p),
                source,
                target,
            } => {
                if declared.insert(*p) {
                    out.push([*p, ty, owl_obj]);
                }
                out.push([*source, *p, *target]);
            }
            other => panic!("render_flat only supports the flat subset, got {:?}", other),
        }
    }
    out
}

#[test]
fn round_trip_flat_subset() {
    let (mut d, t) =
        parse(":A rdfs:subClassOf :B .\n:x a :A .\n:p a owl:ObjectProperty .\n:x :p :y .");
    let onto1 = extract(&d, &t).expect("accept");
    let t2 = render_flat(&mut d, &onto1);
    let onto2 = extract(&d, &t2).expect("re-extract");
    assert_eq!(onto1.axioms, onto2.axioms, "model → RDF → model must be identity");
    // Sanity: the flat subset really was present.
    assert_eq!(onto1.len(), 3);
}

// ============================== REJECT (one per taxonomy arm) ==============================

#[test]
fn reject_out_of_fragment_cardinality() {
    let (d, t) = parse(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ; owl:maxCardinality 1 ] .",
    );
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::OutOfFragment(_))
    ));
}

#[test]
fn reject_out_of_fragment_sameas() {
    let (d, t) = parse(":x owl:sameAs :y .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::OutOfFragment(_))
    ));
}

#[test]
fn reject_out_of_fragment_inverse() {
    let (d, t) = parse(":r owl:inverseOf :s .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::OutOfFragment(_))
    ));
}

#[test]
fn reject_out_of_fragment_nominal_oneof() {
    let (d, t) = parse(":A owl:oneOf ( :a :b ) .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::OutOfFragment(_))
    ));
}

/// With the opt-in `dl_transitive` feature OFF (the default), `owl:TransitiveProperty`
/// stays a fail-closed out-of-fragment refusal — byte-identical to the pre-sq-zfwzq
/// behaviour. (The feature-ON extraction is pinned in the `transitive` module below.)
#[cfg(not(feature = "dl_transitive"))]
#[test]
fn reject_out_of_fragment_property_characteristic() {
    let (d, t) = parse(":p a owl:TransitiveProperty .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::OutOfFragment(_))
    ));
}

/// [SONNET-4.6] sq-pbz04.4.8 — OWL 2's built-in properties have a FIXED extension
/// (`owl:topObjectProperty` = ΔI × ΔI, `owl:bottomObjectProperty` = ∅). L1 can only
/// represent axiom-constrained named roles, so reading one as an ordinary role does not
/// merely lose precision — it manufactures models. Refused fail-closed at EVERY property
/// position (the uniformity is what makes the refusal sound).
#[test]
fn reject_builtin_object_properties_uniformly() {
    // The two W3C cases that motivated the refusal, verbatim in shape. Both are
    // INCONSISTENT in OWL 2 DL; under an opaque-role reading both look satisfiable.
    for body in [
        // New-Feature-BottomObjectProperty-001: ∃⊥ᵣ.⊤(i) — unsatisfiable, ⊥ᵣ is empty.
        ":i a [ owl:onProperty owl:bottomObjectProperty ; owl:someValuesFrom owl:Thing ] .",
        // New-Feature-TopObjectProperty-001: ¬∃⊤ᵣ.⊤(i) — unsatisfiable, ⊤ᵣ relates all.
        ":i a [ owl:complementOf [ owl:onProperty owl:topObjectProperty ; \
                                   owl:someValuesFrom owl:Thing ] ] .",
        // The other property positions, so the refusal cannot be routed around.
        ":r rdfs:subPropertyOf owl:topObjectProperty .",
        "owl:topObjectProperty rdfs:subPropertyOf :r .",
        "owl:bottomObjectProperty rdfs:domain :A .",
        "owl:bottomObjectProperty rdfs:range :A .",
        // The role-assertion path builds its property expression directly, NOT through
        // decode_object_property — a refusal that missed this site would be non-uniform.
        // DECLARED, so the triple gets past the undeclared-predicate refusal and actually
        // reaches `role_assertion` (undeclared, it fails closed earlier as Unclassifiable —
        // asserted separately below so this case cannot pass for the wrong reason).
        "owl:topObjectProperty a owl:ObjectProperty . :x owl:topObjectProperty :y .",
    ] {
        let (d, t) = parse(body);
        assert!(
            matches!(extract(&d, &t), Err(ExtractError::OutOfFragment(_))),
            "expected OutOfFragment for {:?}, got {:?}",
            body,
            extract(&d, &t)
        );
    }
    // UNDECLARED, the built-in predicate is already refused a step earlier (a bare predicate
    // is indistinguishable from an annotation) — pinned so the case above is known to be
    // testing `role_assertion`'s refusal rather than inheriting this one.
    let (d, t) = parse(":x owl:topObjectProperty :y .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::Unclassifiable(_))
    ));
    // The data-property counterparts refuse as DataConstruct (no concrete domain in L1).
    let (d, t) = parse("owl:topDataProperty a owl:ObjectProperty . :x owl:topDataProperty :y .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::DataConstruct(_))
    ));
    // CONTROL: an ordinary role in the same shapes still extracts — the refusal is keyed on
    // the built-in IRIs, not on the shapes carrying them.
    let (d, t) = parse(":i a [ owl:onProperty :r ; owl:someValuesFrom owl:Thing ] . :x :r :y .");
    assert!(extract(&d, &t).is_ok());
}

#[test]
fn reject_data_construct_literal_in_class() {
    let (d, t) = parse(":A rdfs:subClassOf \"a literal\" .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::DataConstruct(_))
    ));
}

#[test]
fn reject_data_construct_data_property() {
    let (d, t) = parse(":p a owl:DatatypeProperty .\n:x :p \"30\"^^xsd:integer .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::DataConstruct(_))
    ));
}

#[test]
fn reject_malformed_list_unterminated() {
    // A list cell with rdf:first but no rdf:rest.
    let (d, t) = parse(":C rdfs:subClassOf _:x .\n_:x owl:intersectionOf _:l .\n_:l rdf:first :A .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::MalformedList(_))
    ));
}

#[test]
fn reject_malformed_list_empty() {
    let (d, t) = parse(":C rdfs:subClassOf [ owl:intersectionOf ( ) ] .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::MalformedList(_))
    ));
}

#[test]
fn reject_malformed_restriction_missing_on_property() {
    let (d, t) = parse(":A rdfs:subClassOf [ a owl:Restriction ; owl:someValuesFrom :C ] .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::MalformedClassExpression(_))
    ));
}

#[test]
fn reject_malformed_restriction_missing_filler() {
    let (d, t) = parse(":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ] .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::MalformedClassExpression(_))
    ));
}

#[test]
fn reject_unclassifiable_undeclared_predicate() {
    // :x :undeclared :y — :undeclared is neither a known annotation property nor a declared
    // object property, and :y is an IRI: a role assertion and an annotation are indistinguishable.
    let (d, t) = parse(":x :undeclared :y .");
    assert!(matches!(
        extract(&d, &t),
        Err(ExtractError::Unclassifiable(_))
    ));
}

// ============================== REJECT (Fix 1: duplicate backbone edges — branching / double-define) ==============================

/// A branching list: the same list cell carries two DIFFERENT rdf:first values.
/// Before fix 1 the second value silently overwrote the first, dropping the constraint.
/// After fix 1 this must be refused with MalformedList. [OPUS-4.8]
#[test]
fn reject_branching_list_rdf_first() {
    let (d, t) = parse(
        ":C rdfs:subClassOf _:x .\n\
         _:x owl:intersectionOf _:l .\n\
         _:l rdf:first :A .\n\
         _:l rdf:first :B .\n\
         _:l rdf:rest rdf:nil .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedList(_))),
        "branching rdf:first must be refused"
    );
}

/// A named class carrying two different owl:intersectionOf heads — duplicate backbone predicate.
/// Before fix 1 the second list head silently overwrote the first.
#[test]
fn reject_duplicate_intersection_of_definition() {
    // Turtle list syntax generates fresh blank nodes for each list,
    // so the two heads are distinct ids — the duplicate-value check fires.
    let (d, t) = parse(
        ":C rdfs:subClassOf :A .\n\
         :A owl:intersectionOf ( :B :D ) .\n\
         :A owl:intersectionOf ( :E :F ) .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedClassExpression(_))),
        "double owl:intersectionOf on a named class must be refused"
    );
}

/// A restriction node carrying two different owl:someValuesFrom fillers.
#[test]
fn reject_duplicate_some_values_from() {
    let (d, t) = parse(
        ":A rdfs:subClassOf _:r .\n\
         _:r owl:onProperty :p .\n\
         _:r owl:someValuesFrom :B .\n\
         _:r owl:someValuesFrom :C .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedClassExpression(_))),
        "duplicate someValuesFrom must be refused"
    );
}

/// A restriction node carrying two different owl:onProperty values.
#[test]
fn reject_duplicate_on_property() {
    let (d, t) = parse(
        ":A rdfs:subClassOf _:r .\n\
         _:r owl:onProperty :p .\n\
         _:r owl:onProperty :q .\n\
         _:r owl:someValuesFrom :B .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedClassExpression(_))),
        "duplicate owl:onProperty must be refused"
    );
}

// ============================== REJECT (Fix 2: blank node in object-property position) ==============================

/// owl:onProperty _:bp — a blank node is not a named object property.
/// Before fix 2 this was accepted as an opaque named property.
/// After fix 2 it must be refused with Unclassifiable. [OPUS-4.8]
#[test]
fn reject_blank_node_object_property_on_property() {
    let (d, t) = parse(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty _:bp ; owl:someValuesFrom :C ] .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "blank node in owl:onProperty position must be refused"
    );
}

/// _:bp rdfs:subPropertyOf :q — a blank node as the sub-property subject is not a named property.
#[test]
fn reject_blank_node_sub_property_of() {
    let (d, t) = parse("_:bp rdfs:subPropertyOf :q .");
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "blank node in rdfs:subPropertyOf subject position must be refused"
    );
}

// ============================== REJECT (Fix 3: punned predicate) ==============================

/// :p declared as both AnnotationProperty AND ObjectProperty, then used in :a :p :b.
/// Before fix 3 the annotation arm silently dropped the triple (wrong classification).
/// After fix 3 this must be refused with Unclassifiable. [OPUS-4.8]
#[test]
fn reject_punned_annotation_and_object_property() {
    let (d, t) = parse(
        ":p a owl:AnnotationProperty .\n\
         :p a owl:ObjectProperty .\n\
         :a :p :b .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "predicate declared as both AnnotationProperty and ObjectProperty must be refused"
    );
}

// ============================== REJECT (soundness closure: punned property in axiom-arg position) ==============================
// [SONNET-4.6] sq-pbz04.4.1: the original Fix-3 check only covered the role-assertion
// fall-through path. The axiom-dispatch arms (domain/range/subPropertyOf) and decode_restriction
// (owl:onProperty) each call decode_object_property, which is now the structural chokepoint.
// These four tests confirm that a punned property is refused in EVERY axiom-argument position.

/// Punned :p (AnnotationProperty + ObjectProperty) in rdfs:domain subject position.
/// Must be refused — ObjectPropertyDomain(:p, :C) must NOT be produced. [SONNET-4.6]
#[test]
fn reject_punned_property_in_domain() {
    let (d, t) = parse(
        ":p a owl:ObjectProperty .\n\
         :p a owl:AnnotationProperty .\n\
         :p rdfs:domain :C .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "punned property in rdfs:domain subject position must be refused"
    );
}

/// Punned :p (AnnotationProperty + ObjectProperty) in rdfs:range subject position.
/// Must be refused — ObjectPropertyRange(:p, :C) must NOT be produced. [SONNET-4.6]
#[test]
fn reject_punned_property_in_range() {
    let (d, t) = parse(
        ":p a owl:ObjectProperty .\n\
         :p a owl:AnnotationProperty .\n\
         :p rdfs:range :C .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "punned property in rdfs:range subject position must be refused"
    );
}

/// Punned :p (AnnotationProperty + ObjectProperty) in rdfs:subPropertyOf subject AND object
/// positions (two separate triples). Both must be refused. [SONNET-4.6]
#[test]
fn reject_punned_property_in_sub_property_of() {
    // Subject position: :p rdfs:subPropertyOf :q — :p is punned.
    let (d, t) = parse(
        ":p a owl:ObjectProperty .\n\
         :p a owl:AnnotationProperty .\n\
         :q a owl:ObjectProperty .\n\
         :p rdfs:subPropertyOf :q .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "punned property in rdfs:subPropertyOf subject position must be refused"
    );

    // Object position: :q rdfs:subPropertyOf :p — :p is punned.
    let (d, t) = parse(
        ":p a owl:ObjectProperty .\n\
         :p a owl:AnnotationProperty .\n\
         :q a owl:ObjectProperty .\n\
         :q rdfs:subPropertyOf :p .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "punned property in rdfs:subPropertyOf object position must be refused"
    );
}

/// Punned :p (AnnotationProperty + ObjectProperty) in owl:onProperty object position inside a
/// restriction. Must be refused — ObjectSomeValuesFrom(:p, :C) must NOT be produced. [SONNET-4.6]
#[test]
fn reject_punned_property_in_on_property() {
    let (d, t) = parse(
        ":p a owl:ObjectProperty .\n\
         :p a owl:AnnotationProperty .\n\
         :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :C ] .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "punned property in owl:onProperty position must be refused"
    );
}

// ============================== ACCEPT (control: unambiguous property in each axiom-arg position) ==============================
// [SONNET-4.6] sq-pbz04.4.1: confirm the fix does not over-reject an unambiguous ObjectProperty.

/// An unambiguously declared ObjectProperty in rdfs:domain produces ObjectPropertyDomain. [SONNET-4.6]
#[test]
fn accept_unambiguous_property_in_domain() {
    let (d, t) = parse(":p a owl:ObjectProperty .\n:p rdfs:domain :C .");
    let o = extract(&d, &t).expect("accept");
    assert!(
        o.axioms.iter().any(|ax| matches!(ax, Axiom::ObjectPropertyDomain { .. })),
        "unambiguous ObjectProperty in rdfs:domain must produce ObjectPropertyDomain"
    );
}

/// An unambiguously declared ObjectProperty in rdfs:range produces ObjectPropertyRange. [SONNET-4.6]
#[test]
fn accept_unambiguous_property_in_range() {
    let (d, t) = parse(":p a owl:ObjectProperty .\n:p rdfs:range :C .");
    let o = extract(&d, &t).expect("accept");
    assert!(
        o.axioms.iter().any(|ax| matches!(ax, Axiom::ObjectPropertyRange { .. })),
        "unambiguous ObjectProperty in rdfs:range must produce ObjectPropertyRange"
    );
}

/// An unambiguously declared ObjectProperty in rdfs:subPropertyOf produces SubObjectPropertyOf. [SONNET-4.6]
#[test]
fn accept_unambiguous_property_in_sub_property_of() {
    let (d, t) = parse(
        ":p a owl:ObjectProperty .\n\
         :q a owl:ObjectProperty .\n\
         :p rdfs:subPropertyOf :q .",
    );
    let o = extract(&d, &t).expect("accept");
    assert!(
        o.axioms.iter().any(|ax| matches!(ax, Axiom::SubObjectPropertyOf { .. })),
        "unambiguous ObjectProperty in rdfs:subPropertyOf must produce SubObjectPropertyOf"
    );
}

/// An unambiguous restriction over a declared ObjectProperty produces ObjectSomeValuesFrom. [SONNET-4.6]
#[test]
fn accept_unambiguous_property_in_on_property() {
    let (d, t) = parse(
        ":p a owl:ObjectProperty .\n\
         :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :C ] .",
    );
    let o = extract(&d, &t).expect("accept");
    assert!(
        o.axioms.iter().any(|ax| matches!(
            ax,
            Axiom::SubClassOf {
                sup: sparq_reason_dl::model::ClassExpression::ObjectSomeValuesFrom(..),
                ..
            }
        )),
        "unambiguous ObjectProperty in owl:onProperty must produce ObjectSomeValuesFrom"
    );
}

// ============================== ACCEPT (Fix 4: owl:Deprecated* meta-classes) ==============================

/// owl:DeprecatedClass is a structural meta-class (OWL 2 §11.2) — no logical axioms.
/// Before fix 4 this produced a spurious ClassAssertion. [OPUS-4.8]
#[test]
fn accept_deprecated_class_as_declaration() {
    let (d, t) = parse(":A a owl:DeprecatedClass .");
    let o = extract(&d, &t).expect("accept");
    assert!(o.is_empty(), "owl:DeprecatedClass must be ignored, got {:?}", o.axioms);
}

/// owl:DeprecatedProperty is a structural meta-class — no logical axioms.
#[test]
fn accept_deprecated_property_as_declaration() {
    let (d, t) = parse(":p a owl:DeprecatedProperty .");
    let o = extract(&d, &t).expect("accept");
    assert!(o.is_empty(), "owl:DeprecatedProperty must be ignored, got {:?}", o.axioms);
}

// ============================== REJECT (existing guards, now explicitly pinned) ==============================

/// Direct self-referential cycle: _:x owl:complementOf _:x.
/// The cycle guard (visiting set) must catch this. [OPUS-4.8] pinned
#[test]
fn reject_cyclic_class_expression() {
    let (d, t) = parse(
        ":A rdfs:subClassOf _:x .\n\
         _:x owl:complementOf _:x .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedClassExpression(_))),
        "cyclic complementOf must be refused"
    );
}

/// A complementOf chain 513 levels deep exceeds MAX_CE_DEPTH (512): must produce
/// MalformedClassExpression rather than a stack overflow. [OPUS-4.8] pinned
#[test]
fn reject_max_ce_depth() {
    let mut ttl = String::from(":A rdfs:subClassOf _:n0 .\n");
    for i in 0..513usize {
        ttl.push_str(&format!("_:n{} owl:complementOf _:n{} .\n", i, i + 1));
    }
    // _:n513 is the leaf; the depth check fires when decode_class_inner is called at
    // depth 513 (513 > MAX_CE_DEPTH=512), before any properties of _:n513 are inspected.
    let (d, t) = parse(&ttl);
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedClassExpression(_))),
        "expression exceeding MAX_CE_DEPTH must be refused"
    );
}

/// A node simultaneously carrying owl:intersectionOf AND owl:unionOf shapes — shape_count > 1.
/// [OPUS-4.8] pinned
#[test]
fn reject_multi_shape_node() {
    let (d, t) = parse(
        ":C rdfs:subClassOf _:x .\n\
         _:x owl:intersectionOf ( :A :B ) .\n\
         _:x owl:unionOf ( :D :E ) .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedClassExpression(_))),
        "node with multiple class-expression shapes must be refused"
    );
}

/// A restriction carrying BOTH someValuesFrom AND allValuesFrom — decode_restriction rejects.
/// [OPUS-4.8] pinned
#[test]
fn reject_restriction_both_some_and_all_values_from() {
    let (d, t) = parse(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ; \
         owl:someValuesFrom :B ; owl:allValuesFrom :C ] .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedClassExpression(_))),
        "restriction with both someValuesFrom and allValuesFrom must be refused"
    );
}

/// A class-expression / restriction blank node used as the TARGET of a role assertion —
/// role_assertion checks structural_nodes. [OPUS-4.8] pinned
#[test]
fn reject_structural_node_used_as_individual() {
    let (d, t) = parse(
        ":p a owl:ObjectProperty .\n\
         :x :p _:r .\n\
         _:r a owl:Restriction .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedClassExpression(_))),
        "structural (restriction) node used as a role-assertion individual must be refused"
    );
}

// ============================== REJECT (order-asymmetry in punning detection) ==============================
// [SONNET-4.6] sq-pbz04.4.1: Index::build usage-typing arms must detect a pun regardless of
// whether the conflicting declaration appears BEFORE or AFTER the usage triple (RDF graphs are
// sets — triple ordering must not affect the verdict).

/// ORDER: AnnotationProperty declaration FIRST, then owl:onProperty use (the formerly-broken
/// order). Before the order-asymmetry fix, annotation_props got the IRI and the later usage arm
/// added it to object_props without checking annotation_props → pun undetected → ACCEPTED. [SONNET-4.6]
#[test]
fn reject_pun_declare_annotation_then_use_on_property() {
    let (d, t) = parse(
        ":p a owl:AnnotationProperty .\n\
         :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :C ] .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "pun (declare-annotation-then-use) must be refused regardless of triple order"
    );
}

/// ORDER: owl:onProperty use FIRST, then AnnotationProperty declaration (direction already
/// caught by the declaration arm). Pinned as the symmetric pair. [SONNET-4.6]
#[test]
fn reject_pun_use_then_declare_annotation_on_property() {
    let (d, t) = parse(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :C ] .\n\
         :p a owl:AnnotationProperty .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "pun (use-then-declare-annotation) must be refused regardless of triple order"
    );
}

/// Same two-order test with DatatypeProperty (F2). [SONNET-4.6]
#[test]
fn reject_pun_declare_datatype_then_use_on_property() {
    let (d, t) = parse(
        ":p a owl:DatatypeProperty .\n\
         :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :C ] .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "pun (declare-datatype-then-use) must be refused regardless of triple order"
    );
}

#[test]
fn reject_pun_use_then_declare_datatype_on_property() {
    let (d, t) = parse(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :C ] .\n\
         :p a owl:DatatypeProperty .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::Unclassifiable(_))),
        "pun (use-then-declare-datatype) must be refused regardless of triple order"
    );
}

/// Triple-order permutation test: a punned AnnotationProperty used in owl:onProperty must be
/// refused with Unclassifiable under at least 3 orderings of the triples slice. [SONNET-4.6]
#[test]
fn extract_verdict_order_independent_pun_permutation() {
    let (d, mut t) = parse(
        ":p a owl:AnnotationProperty .\n\
         :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :C ] .",
    );
    let kind = |r: Result<Ontology, ExtractError>| match r {
        Err(ExtractError::Unclassifiable(_)) => "unclassifiable",
        Err(_) => "other_err",
        Ok(_) => "ok",
    };
    let v0 = kind(extract(&d, &t));
    t.sort();
    let v1 = kind(extract(&d, &t));
    t.sort_by(|a, b| b.cmp(a));
    let v2 = kind(extract(&d, &t));
    assert_eq!("unclassifiable", v0, "original order must refuse as Unclassifiable");
    assert_eq!("unclassifiable", v1, "sorted-asc order must refuse as Unclassifiable");
    assert_eq!("unclassifiable", v2, "sorted-desc order must refuse as Unclassifiable");
}

// ============================== REJECT (bare blank in class-expression position) ==============================
// [SONNET-4.6] sq-pbz04.4.1 (Copilot thread 4): a blank node in a class-expression position
// with no class-expression backbone predicates has no sound OWL mapping and must be refused.

/// :A rdfs:subClassOf _:b with NO triples about _:b previously yielded Ok(Class(_:b)) — an
/// opaque blank that the downstream checker would treat as a real class constant. Now refused. [SONNET-4.6]
#[test]
fn reject_bare_blank_in_class_expression_position() {
    // _:b carries no backbone predicates — no owl:intersectionOf / owl:unionOf /
    // owl:complementOf / owl:Restriction typing. A bare blank in subClassOf object position.
    let (d, t) = parse(":A rdfs:subClassOf _:b .");
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedClassExpression(_))),
        "bare blank node in class-expression position must be refused as MalformedClassExpression"
    );
}

/// Control: a blank node WITH a proper backbone (owl:complementOf) still decodes. [SONNET-4.6]
#[test]
fn accept_blank_with_backbone_in_class_expression_position() {
    let (d, t) = parse(":A rdfs:subClassOf [ owl:complementOf :B ] .");
    assert!(
        extract(&d, &t).is_ok(),
        "blank node with owl:complementOf backbone must still decode"
    );
}

// ============================== REJECT (inverse-property-expression pin) ==============================
// [SONNET-4.6] sq-pbz04.4.1 (Copilot threads 2/3): owl:onProperty [ owl:inverseOf :r ] is
// refused as OutOfFragment. The owl:inverseOf :r triple's PREDICATE fires in classify_triple's
// out_of_fragment arm before any restriction decoding is attempted, so the check order inside
// decode_object_property (blank-before-inverseOf) does not affect the overall verdict.

/// Pin test: owl:onProperty [ owl:inverseOf :r ] → OutOfFragment. The owl:inverseOf predicate
/// is in classify_triple's out_of_fragment map; it fires before decode_restriction is reached.
/// This makes the blank-before-inverseOf concern in decode_object_property moot for this graph
/// shape (Copilot threads 2/3 empirically refuted). [SONNET-4.6]
#[test]
fn reject_inverse_property_expression_in_on_property_pin() {
    let (d, t) = parse(
        ":A rdfs:subClassOf [ a owl:Restriction ; \
         owl:onProperty [ owl:inverseOf :r ] ; \
         owl:someValuesFrom :C ] .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::OutOfFragment(_))),
        "owl:onProperty [ owl:inverseOf :r ] must be refused as OutOfFragment (via classify_triple)"
    );
}

// ============================== FAIL-CLOSED INVARIANT ==============================

#[test]
fn fail_closed_rejects_whole_graph_on_one_bad_triple() {
    // The load-bearing soundness invariant: a graph that is 99% in-fragment but contains ONE
    // out-of-fragment triple is REFUSED whole — never returned as a partial model that silently
    // dropped the offending axiom (which could flip a downstream consistency verdict).
    let (d, t) = parse(":A rdfs:subClassOf :B .\n:B rdfs:subClassOf :C .\n:x owl:sameAs :y .");
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::OutOfFragment(_))),
        "one out-of-fragment triple must reject the whole graph"
    );
}

// ============================== sq-pbz04.4.11 — named-composite EquivalentClasses (M1 fix) ======

// [SONNET-4.6] sq-pbz04.4.11: the M1 fidelity gap — a NAMED class carrying an inline backbone
// definition must emit EquivalentClasses(name, expr) so entailment through the name works.
// These tests are the acceptance tests for the 12 M1 divergence cases identified by the
// conformance arm in tests/dl_suite.rs.

/// A NAMED class carrying an owl:intersectionOf definition must produce
/// EquivalentClasses(A, x⊓y) — the M1 fix for intersection cases.
/// [SONNET-4.6] sq-pbz04.4.11
#[test]
fn named_class_intersection_emits_equivalent_classes() {
    // :A owl:intersectionOf (:B :C) — A is named; must produce EquivalentClasses(A, B⊓C).
    let (d, t) = parse(":A owl:intersectionOf ( :B :C ) .");
    let o = extract(&d, &t).expect("accept");
    let a = class(&d, "A");
    let expected_expr = CE::ObjectIntersectionOf(vec![class(&d, "B"), class(&d, "C")]);
    assert!(
        o.axioms.contains(&Axiom::EquivalentClasses(a.clone(), expected_expr.clone())),
        "named intersectionOf must emit EquivalentClasses(A, B⊓C); got {:?}",
        o.axioms
    );
    // Exactly one axiom: the name-binding.
    assert_eq!(
        o.len(),
        1,
        "only the EquivalentClasses name-binding should be emitted; got {:?}",
        o.axioms
    );
}

// -------------------------------------------------------------------------------------------
// Opt-in transitive roles ([GPT-5.6] sq-zfwzq, feature `dl_transitive`)
// -------------------------------------------------------------------------------------------

#[cfg(feature = "dl_transitive")]
mod transitive {
    use super::*;
    use sparq_reason_dl::model::ObjectPropertyExpression;

    /// With the feature ON, `owl:TransitiveProperty` EXTRACTS as
    /// `Axiom::TransitiveObjectProperty` on the named property — no longer refused.
    #[test]
    fn transitive_property_extracts_as_axiom() {
        let (d, t) = parse(":p a owl:TransitiveProperty .");
        let onto = extract(&d, &t).expect("in-fragment under dl_transitive");
        let p = ex(&d, "p");
        assert_eq!(
            onto.axioms,
            vec![Axiom::TransitiveObjectProperty {
                property: ObjectPropertyExpression::ObjectProperty(p),
            }]
        );
    }

    /// The transitivity typing also TYPES the subject as an object property, so a role
    /// assertion over it classifies without a separate `owl:ObjectProperty` declaration.
    #[test]
    fn transitivity_typing_enables_role_assertion_classification() {
        let (d, t) = parse(":p a owl:TransitiveProperty .\n:a :p :b .");
        let onto = extract(&d, &t).expect("extracts");
        assert!(onto
            .axioms
            .iter()
            .any(|a| matches!(a, Axiom::ObjectPropertyAssertion { .. })));
        assert!(onto
            .axioms
            .iter()
            .any(|a| matches!(a, Axiom::TransitiveObjectProperty { .. })));
    }

    /// Fail-closed edges stay closed: a BLANK-node subject of the transitivity typing is
    /// refused (named object properties only in L1), and a cross-type punned subject
    /// (also declared AnnotationProperty) is Unclassifiable — in EITHER assertion order.
    #[test]
    fn transitivity_on_blank_or_punned_subject_refuses() {
        let (d, t) = parse("_:b a owl:TransitiveProperty .");
        assert!(matches!(
            extract(&d, &t),
            Err(ExtractError::Unclassifiable(_))
        ));
        let (d, t) = parse(":p a owl:AnnotationProperty .\n:p a owl:TransitiveProperty .");
        assert!(matches!(
            extract(&d, &t),
            Err(ExtractError::Unclassifiable(_))
        ));
        let (d, t) = parse(":p a owl:TransitiveProperty .\n:p a owl:AnnotationProperty .");
        assert!(matches!(
            extract(&d, &t),
            Err(ExtractError::Unclassifiable(_))
        ));
    }

    /// The OTHER property characteristics remain out-of-fragment refusals under the
    /// feature — the opt-in admits transitivity ONLY.
    #[test]
    fn other_property_characteristics_still_refused() {
        for ttl in [
            ":p a owl:FunctionalProperty .",
            ":p a owl:SymmetricProperty .",
            ":p a owl:ReflexiveProperty .",
        ] {
            let (d, t) = parse(ttl);
            assert!(
                matches!(extract(&d, &t), Err(ExtractError::OutOfFragment(_))),
                "{} must stay refused",
                ttl
            );
        }
    }
}

/// A NAMED class carrying an owl:unionOf definition must produce
/// EquivalentClasses(A, x⊔y) — the M1 fix for union cases.
/// [SONNET-4.6] sq-pbz04.4.11
#[test]
fn named_class_union_emits_equivalent_classes() {
    let (d, t) = parse(":A owl:unionOf ( :Human :Animal ) .");
    let o = extract(&d, &t).expect("accept");
    let a = class(&d, "A");
    let expected_expr = CE::ObjectUnionOf(vec![class(&d, "Human"), class(&d, "Animal")]);
    assert!(
        o.axioms.contains(&Axiom::EquivalentClasses(a, expected_expr)),
        "named unionOf must emit EquivalentClasses(A, Human⊔Animal); got {:?}",
        o.axioms
    );
}

/// A NAMED class carrying an owl:complementOf definition must produce
/// EquivalentClasses(A, ¬B).
/// [SONNET-4.6] sq-pbz04.4.11
#[test]
fn named_class_complement_emits_equivalent_classes() {
    let (d, t) = parse(":A owl:complementOf :B .");
    let o = extract(&d, &t).expect("accept");
    let a = class(&d, "A");
    let expected_expr = CE::ObjectComplementOf(Box::new(class(&d, "B")));
    assert!(
        o.axioms.contains(&Axiom::EquivalentClasses(a, expected_expr)),
        "named complementOf must emit EquivalentClasses(A, ¬B); got {:?}",
        o.axioms
    );
}

/// A NAMED class carrying an owl:Restriction backbone must produce
/// EquivalentClasses(A, ∃r.C) — the M1 fix for restriction cases.
/// [SONNET-4.6] sq-pbz04.4.11
#[test]
fn named_class_restriction_backbone_emits_equivalent_classes() {
    // :z a owl:Restriction; owl:onProperty :p; owl:someValuesFrom :C  (z is named IRI)
    let (d, t) = parse(":z a owl:Restriction .\n:z owl:onProperty :p .\n:z owl:someValuesFrom :C .");
    let o = extract(&d, &t).expect("accept");
    let z = class(&d, "z");
    let expected_expr = CE::ObjectSomeValuesFrom(prop(&d, "p"), Box::new(class(&d, "C")));
    assert!(
        o.axioms.contains(&Axiom::EquivalentClasses(z, expected_expr)),
        "named restriction must emit EquivalentClasses(z, ∃p.C); got {:?}",
        o.axioms
    );
}

/// The name-binding EquivalentClasses axiom must be PREPENDED before any use-site axioms,
/// so the structural model sees "define name, then use name". This is the ordering contract
/// the downstream tableau depends on.
/// [SONNET-4.6] sq-pbz04.4.11
#[test]
fn named_composite_equiv_prepended_before_use_site_axioms() {
    // :A owl:intersectionOf (:B :C); then a subClassOf using :A.
    let (d, t) = parse(
        ":A owl:intersectionOf ( :B :C ) .\n\
         :D rdfs:subClassOf :A .",
    );
    let o = extract(&d, &t).expect("accept");
    assert!(o.len() >= 2, "expected at least 2 axioms, got {:?}", o.axioms);
    // The FIRST axiom must be the name-binding EquivalentClasses.
    assert!(
        matches!(&o.axioms[0], Axiom::EquivalentClasses(CE::Class(_), _)),
        "first axiom must be EquivalentClasses (name-binding); got {:?}",
        o.axioms[0]
    );
}

/// A BLANK node carrying a backbone definition must NOT emit EquivalentClasses —
/// only NAMED classes (IRIs) get the binding; anonymous nodes are just inline expressions.
/// [SONNET-4.6] sq-pbz04.4.11
#[test]
fn blank_node_backbone_does_not_emit_equivalent_classes() {
    // _:x owl:intersectionOf (:A :B) used in :D rdfs:subClassOf _:x — anonymous expression.
    let (d, t) = parse(":D rdfs:subClassOf [ owl:intersectionOf ( :A :B ) ] .");
    let o = extract(&d, &t).expect("accept");
    // The only axiom must be the SubClassOf (the anonymous blank carries no binding).
    assert_eq!(o.len(), 1, "blank backbone must yield only SubClassOf, got {:?}", o.axioms);
    assert!(
        matches!(&o.axioms[0], Axiom::SubClassOf { .. }),
        "sole axiom must be SubClassOf, got {:?}",
        o.axioms[0]
    );
    // Verify: no EquivalentClasses axiom.
    for ax in &o.axioms {
        assert!(
            !matches!(ax, Axiom::EquivalentClasses(..)),
            "blank backbone must NOT produce EquivalentClasses"
        );
    }
}

/// The M1 fix must not regress the existing union-name case: a named union class
/// (like `A owl:unionOf (Human Animal)`) used with a ClassAssertion must now
/// allow the downstream checker to derive A(John) from Human(John) — the
/// EquivalentClasses binding is what makes that entailment possible.
/// [SONNET-4.6] sq-pbz04.4.11
#[test]
fn named_union_with_class_assertion_emits_both_equiv_and_assertion() {
    // :A owl:unionOf (:Human :Animal) — named union; :John a :Human.
    let (d, t) = parse(
        ":A owl:unionOf ( :Human :Animal ) .\n\
         :John a :Human .",
    );
    let o = extract(&d, &t).expect("accept");
    let a = class(&d, "A");
    let equiv = Axiom::EquivalentClasses(
        a,
        CE::ObjectUnionOf(vec![class(&d, "Human"), class(&d, "Animal")]),
    );
    let assertion = Axiom::ClassAssertion {
        class: class(&d, "Human"),
        individual: ex(&d, "John"),
    };
    assert!(
        o.axioms.contains(&equiv),
        "EquivalentClasses(A, Human⊔Animal) must be present; got {:?}",
        o.axioms
    );
    assert!(
        o.axioms.contains(&assertion),
        "ClassAssertion(Human, John) must be present; got {:?}",
        o.axioms
    );
    // The EquivalentClasses axiom must come BEFORE the ClassAssertion (name binding first).
    let equiv_pos = o.axioms.iter().position(|ax| ax == &equiv).unwrap();
    let assert_pos = o.axioms.iter().position(|ax| ax == &assertion).unwrap();
    assert!(
        equiv_pos < assert_pos,
        "EquivalentClasses must precede ClassAssertion (position {} vs {})",
        equiv_pos, assert_pos
    );
}

// ============================== REJECT (M4: orphan/cyclic list and unconsumed backbone) ======
// [OPUS-4.8] sq-pbz04.4.12: the M4 fail-closed fix — orphan/cyclic rdf:first/rdf:rest cells
// and unconsumed class-expression backbones must be REFUSED, not silently treated as inert.
// These four cases correspond to the four M4 W3C Direct-Semantics corpus divergences:
// I5.5-003, I5.5-004, I5.5-006, I5.5-007.
//
// Soundness invariant: an empty or partial extraction from a malformed/cyclic backbone is an
// UNSOUND basis for entailment — the downstream reasoner must never reason over such a model.

/// rdf:nil rdf:rest _:b — `rdf:nil` cannot be a list cell (W3C I5.5-003 analog).
/// Before sq-pbz04.4.12, this extracted to an empty ontology (consistent).
/// After: refused as MalformedList. [OPUS-4.8] sq-pbz04.4.12
#[test]
fn reject_rdf_nil_rdf_rest() {
    // Directly build the triple <rdf:nil rdf:rest _:b> in Turtle.
    // In Turtle we must reference rdf:nil explicitly.
    let (d, t) = parse(
        "_:holder rdf:type owl:Class .\n\
         rdf:nil rdf:rest _:b .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedList(_))),
        "rdf:nil rdf:rest _:b must be refused as MalformedList (I5.5-003 analog)"
    );
}

/// rdf:nil rdf:first _:b — `rdf:nil` cannot be a list cell (W3C I5.5-004 analog).
/// Before sq-pbz04.4.12, this extracted to an empty ontology (consistent).
/// After: refused as MalformedList. [OPUS-4.8] sq-pbz04.4.12
#[test]
fn reject_rdf_nil_rdf_first() {
    let (d, t) = parse(
        "_:holder rdf:type owl:Class .\n\
         rdf:nil rdf:first _:b .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedList(_))),
        "rdf:nil rdf:first _:b must be refused as MalformedList (I5.5-004 analog)"
    );
}

/// A cyclic orphan list — `_:l rdf:first :a; rdf:rest _:l` — never consumed by any axiom.
/// FAITHFUL reconstruction of W3C WebOnt-I5.5-006: the corpus non-conclusion is
/// `<rdf:List rdf:nodeID="list"> … <rdf:rest rdf:nodeID="list"/></rdf:List>`, which emits the
/// `_:l rdf:type rdf:List` typing triple. That declaration typing is the BYPASS the [OPUS-4.8]
/// sq-pbz04.4.12 fix closes: it produces no axiom, so it must NOT seed the cyclic cell
/// reachable. Before the fix this (with the typing) extracted to an empty ontology — a WRONG
/// definitive verdict. After: refused as MalformedList (abstention). [OPUS-4.8]
#[test]
fn reject_cyclic_orphan_list_i5_5_006() {
    // A cyclic rdf:List with the `a rdf:List` typing and no owl:intersectionOf/unionOf
    // pointing to it — exactly the corpus WebOnt-I5.5-006 encoding.
    let (d, t) = parse(
        "_:l rdf:type rdf:List .\n\
         _:l rdf:first :a .\n\
         _:l rdf:rest _:l .",
    );
    assert!(
        matches!(extract(&d, &t), Err(ExtractError::MalformedList(_))),
        "cyclic orphan list cell (with `a rdf:List` typing) must be refused as MalformedList \
         (WebOnt-I5.5-006); got {:?}",
        extract(&d, &t)
    );
}

/// FAITHFUL reconstruction of W3C WebOnt-I5.5-007: an anonymous `owl:Class` whose
/// `owl:unionOf` list's first member is an anonymous `owl:Class` with a CYCLIC
/// `owl:intersectionOf` list (`_:il rdf:rest _:il`), and every `rdf:List` node carries the
/// `a rdf:List` typing. None of it is referenced by any axiom predicate. Before [OPUS-4.8]
/// sq-pbz04.4.12 the `a rdf:List` declaration typing seeded the cyclic intersection cell
/// reachable, so the whole graph slipped past the refusal and extracted to an empty ontology
/// (a WRONG definitive verdict). After: refused with MalformedList or MalformedClassExpression
/// (whichever orphan is detected first — both are the correct fail-closed response).
/// [OPUS-4.8] sq-pbz04.4.12
#[test]
fn reject_unconsumed_union_backbone_i5_5_007() {
    // Nested cyclic union/intersection with the `a rdf:List` typings, exactly the corpus
    // WebOnt-I5.5-007 non-conclusion encoding (the outer union head is an anonymous
    // owl:Class; the intersection list `_:il` is self-cyclic via rdf:rest). Nothing here is
    // consumed by an axiom, so it must be refused. Both MalformedClassExpression and
    // MalformedList are correct fail-closed refusals (iteration order over the structural-node
    // set is unspecified).
    let (d, t) = parse(
        "_:u rdf:type owl:Class .\n\
         _:u owl:unionOf _:ul .\n\
         _:ul rdf:type rdf:List .\n\
         _:ul rdf:first _:inner .\n\
         _:ul rdf:rest rdf:nil .\n\
         _:inner rdf:type owl:Class .\n\
         _:inner owl:intersectionOf _:il .\n\
         _:il rdf:type rdf:List .\n\
         _:il rdf:first :a .\n\
         _:il rdf:rest _:il .",
    );
    let result = extract(&d, &t);
    assert!(
        matches!(
            result,
            Err(ExtractError::MalformedClassExpression(_)) | Err(ExtractError::MalformedList(_))
        ),
        "unconsumed cyclic union/intersection backbone (with `a rdf:List` typings) must be \
         refused (WebOnt-I5.5-007); got {:?}",
        result
    );
}

/// Companion minimal case: a single unconsumed anonymous `owl:unionOf` list with the
/// `a rdf:List` typing, ensuring the declaration typing does not rescue the orphan even
/// without the full I5.5-007 nesting. [OPUS-4.8] sq-pbz04.4.12
#[test]
fn reject_unconsumed_union_backbone_typed_list() {
    // _:x owl:unionOf ( :a :b ) where the list head carries `a rdf:List`. _:x is an
    // anonymous blank, never referenced by any axiom predicate.
    let (d, t) = parse(
        "_:x owl:unionOf _:l .\n\
         _:l rdf:type rdf:List .\n\
         _:l rdf:first :a .\n\
         _:l rdf:rest rdf:nil .",
    );
    let result = extract(&d, &t);
    assert!(
        matches!(
            result,
            Err(ExtractError::MalformedClassExpression(_)) | Err(ExtractError::MalformedList(_))
        ),
        "unconsumed anonymous union backbone with a typed list must be refused; got {:?}",
        result
    );
}

/// Control: an anonymous union backbone that IS consumed via a subClassOf still decodes
/// normally. Ensures the new validation does not over-reject valid anonymous expressions.
/// [OPUS-4.8] sq-pbz04.4.12
#[test]
fn accept_consumed_anonymous_union_backbone() {
    // _:x owl:unionOf (:a :b) consumed by :C rdfs:subClassOf _:x.
    let (d, t) = parse(":C rdfs:subClassOf [ owl:unionOf ( :a :b ) ] .");
    assert!(
        extract(&d, &t).is_ok(),
        "anonymous union backbone consumed by an axiom must still be accepted"
    );
}

/// Control: a named class with an inline intersectionOf definition consumed via a use-site
/// axiom — should still produce the EquivalentClasses binding and the use-site axiom.
/// The M4 fix must not regress the M1 fix. [OPUS-4.8] sq-pbz04.4.12
#[test]
fn accept_named_intersection_with_use_site() {
    // :A owl:intersectionOf (:B :C) and :D rdfs:subClassOf :A — consumed.
    let (d, t) = parse(
        ":A owl:intersectionOf ( :B :C ) .\n\
         :D rdfs:subClassOf :A .",
    );
    let o = extract(&d, &t).expect("named intersectionOf with use-site should be accepted");
    assert!(
        o.axioms.iter().any(|ax| matches!(ax, Axiom::EquivalentClasses(CE::Class(_), _))),
        "named intersectionOf must still emit EquivalentClasses; got {:?}",
        o.axioms
    );
}
