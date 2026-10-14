// #7031: the full OWL 2 RDF mapping (`owl2dl::extract`) and the OWL 2 DL profile check
// (`owl2dl::check_dl`).
//
// ACCEPT tests pin the structural model for every class-expression, data-range and axiom
// shape; REJECT tests pin the fail-closed contract (every triple is read by a mapping rule or
// the graph is refused, naming the triple); PROFILE tests pin each OWL 2 DL violation kind.

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_dl::owl2dl::model::{
    AnnotationTarget, Axiom, ClassExpression as CE, DataRange as DR, EntityKind, Individual,
    Literal, ObjectProperty as OP,
};
use sparq_reason_dl::owl2dl::{check_dl, extract, ExtractErrorKind, Ontology, ViolationKind};

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

fn parse(body: &str) -> (Dict, Vec<[Id; 3]>) {
    Graph::parse_to_triples(&format!("{PRE}{body}"), "turtle").expect("parse")
}

fn iri(d: &Dict, full: &str) -> Id {
    d.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(
        full.to_string(),
    )))
}

fn ex(d: &Dict, frag: &str) -> Id {
    iri(d, &format!("http://ex/{frag}"))
}

fn xsd(d: &Dict, local: &str) -> Id {
    iri(d, &format!("{XSD}{local}"))
}

fn lit(lex: &str, local: &str) -> Literal {
    Literal {
        lexical: lex.into(),
        datatype: format!("{XSD}{local}"),
        lang: None,
    }
}

fn ok(body: &str) -> (Dict, Vec<[Id; 3]>, Ontology) {
    let (d, t) = parse(body);
    let o = extract(&d, &t).unwrap_or_else(|e| panic!("expected accept, got {e}"));
    (d, t, o)
}

fn logical(o: &Ontology) -> Vec<Axiom> {
    o.logical_axioms().map(|r| r.axiom.clone()).collect()
}

fn err(body: &str) -> sparq_reason_dl::owl2dl::ExtractError {
    let (d, t) = parse(body);
    match extract(&d, &t) {
        Ok(o) => panic!("expected refusal, got {:?}", logical(&o)),
        Err(e) => e,
    }
}

const DECLS: &str = r#"
    :A a owl:Class . :B a owl:Class . :C a owl:Class .
    :p a owl:ObjectProperty . :q a owl:ObjectProperty . :r a owl:ObjectProperty .
    :d a owl:DatatypeProperty . :e a owl:DatatypeProperty .
"#;

fn with_decls(body: &str) -> String {
    format!("{DECLS}{body}")
}

// ============================== ACCEPT ==============================

/// The volunteering fixture from issue #7031: every construct outside ALCH maps.
#[test]
fn issue_7031_volunteering_fixture() {
    let (d, _, o) = ok(r#"
        @prefix vol: <https://w3id.org/odi/volunteering#> .
        vol:Application a owl:Class . vol:Person a owl:Class . vol:Role a owl:Class .
        vol:TimeCommitment a owl:Class . vol:Agent a owl:Class .
        vol:VolunteeringOpportunity a owl:Class . vol:Session a owl:Class .
        vol:submittedBy a owl:ObjectProperty . vol:hasTimeCommitment a owl:ObjectProperty .
        vol:offeredBy a owl:ObjectProperty . vol:offersOpportunity a owl:ObjectProperty .
        vol:hasLocation a owl:ObjectProperty .
        vol:Application rdfs:subClassOf [ a owl:Restriction ;
            owl:onProperty vol:submittedBy ; owl:onClass vol:Person ;
            owl:qualifiedCardinality "1"^^xsd:nonNegativeInteger ] .
        vol:Role rdfs:subClassOf [ a owl:Restriction ;
            owl:onProperty vol:hasTimeCommitment ; owl:onClass vol:TimeCommitment ;
            owl:maxQualifiedCardinality "1"^^xsd:nonNegativeInteger ] .
        vol:offeredBy owl:inverseOf vol:offersOpportunity .
        vol:forOpportunity a owl:ObjectProperty, owl:FunctionalProperty .
        vol:minimumAge a owl:DatatypeProperty, owl:FunctionalProperty ; rdfs:range xsd:nonNegativeInteger .
        vol:hasLocation rdfs:domain [ owl:unionOf ( vol:VolunteeringOpportunity vol:Session ) ] .
        [ a owl:AllDisjointClasses ; owl:members ( vol:Agent vol:VolunteeringOpportunity vol:Role ) ] .
    "#);
    let v = |l: &str| iri(&d, &format!("https://w3id.org/odi/volunteering#{l}"));
    let ax = logical(&o);
    assert!(ax.contains(&Axiom::SubClassOf(
        CE::Class(v("Application")),
        CE::ExactCardinality(
            1,
            OP::Named(v("submittedBy")),
            Box::new(CE::Class(v("Person")))
        )
    )));
    assert!(ax.contains(&Axiom::SubClassOf(
        CE::Class(v("Role")),
        CE::MaxCardinality(
            1,
            OP::Named(v("hasTimeCommitment")),
            Box::new(CE::Class(v("TimeCommitment")))
        )
    )));
    assert!(ax.contains(&Axiom::InverseObjectProperties(
        OP::Named(v("offeredBy")),
        OP::Named(v("offersOpportunity"))
    )));
    assert!(ax.contains(&Axiom::FunctionalObjectProperty(OP::Named(v(
        "forOpportunity"
    )))));
    assert!(ax.contains(&Axiom::FunctionalDataProperty(v("minimumAge"))));
    assert!(ax.contains(&Axiom::DataPropertyRange(
        v("minimumAge"),
        DR::Datatype(xsd(&d, "nonNegativeInteger"))
    )));
    assert!(ax.contains(&Axiom::ObjectPropertyDomain(
        OP::Named(v("hasLocation")),
        CE::UnionOf(vec![
            CE::Class(v("VolunteeringOpportunity")),
            CE::Class(v("Session"))
        ])
    )));
    assert!(ax.contains(&Axiom::DisjointClasses(vec![
        CE::Class(v("Agent")),
        CE::Class(v("VolunteeringOpportunity")),
        CE::Class(v("Role"))
    ])));
    // vol:forOpportunity / vol:minimumAge are declared by their own typing triples.
    let report = check_dl(&d, &o);
    assert!(report.is_dl(), "{:?}", report.violations);
}

#[test]
fn object_restrictions_and_booleans() {
    let (d, _, o) = ok(&with_decls(
        r#"
        :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :B ] .
        :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty [ owl:inverseOf :p ] ; owl:allValuesFrom :B ] .
        :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:hasValue :a ] .
        :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :q ; owl:hasSelf true ] .
        :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:minCardinality 2 ] .
        :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:maxCardinality "3"^^xsd:nonNegativeInteger ] .
        :B owl:equivalentClass [ a owl:Class ; owl:intersectionOf ( :A [ owl:complementOf :C ] ) ] .
        :C owl:equivalentClass [ a owl:Class ; owl:oneOf ( :a :b ) ] .
    "#,
    ));
    let (a, b, c) = (
        CE::Class(ex(&d, "A")),
        CE::Class(ex(&d, "B")),
        CE::Class(ex(&d, "C")),
    );
    let (p, q) = (OP::Named(ex(&d, "p")), OP::Named(ex(&d, "q")));
    let ax = logical(&o);
    for want in [
        Axiom::SubClassOf(a.clone(), CE::SomeValuesFrom(p, Box::new(b.clone()))),
        Axiom::SubClassOf(
            a.clone(),
            CE::AllValuesFrom(p.inverse(), Box::new(b.clone())),
        ),
        Axiom::SubClassOf(a.clone(), CE::HasValue(p, Individual::Named(ex(&d, "a")))),
        Axiom::SubClassOf(a.clone(), CE::HasSelf(q)),
        Axiom::SubClassOf(a.clone(), CE::MinCardinality(2, p, Box::new(CE::Thing))),
        Axiom::SubClassOf(a.clone(), CE::MaxCardinality(3, p, Box::new(CE::Thing))),
        Axiom::EquivalentClasses(vec![
            b.clone(),
            CE::IntersectionOf(vec![a.clone(), CE::ComplementOf(Box::new(c.clone()))]),
        ]),
        Axiom::EquivalentClasses(vec![
            c.clone(),
            CE::OneOf(vec![
                Individual::Named(ex(&d, "a")),
                Individual::Named(ex(&d, "b")),
            ]),
        ]),
    ] {
        assert!(ax.contains(&want), "missing {want:?} in {ax:?}");
    }
    assert_eq!(ax.len(), 8);
}

#[test]
fn data_restrictions_ranges_and_facets() {
    let (d, _, o) = ok(&with_decls(
        r#"
        :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :d ; owl:someValuesFrom xsd:integer ] .
        :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :d ; owl:allValuesFrom
            [ a rdfs:Datatype ; owl:onDatatype xsd:integer ;
              owl:withRestrictions ( [ xsd:minInclusive 18 ] [ xsd:maxExclusive 120 ] ) ] ] .
        :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :d ; owl:hasValue "x" ] .
        :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :e ; owl:onDataRange xsd:string ;
            owl:maxQualifiedCardinality "1"^^xsd:nonNegativeInteger ] .
        :e rdfs:range [ a rdfs:Datatype ; owl:unionOf ( xsd:string [ a rdfs:Datatype ; owl:datatypeComplementOf xsd:boolean ] ) ] .
        :d rdfs:range [ a rdfs:Datatype ; owl:oneOf ( 1 2 ) ] .
    "#,
    ));
    let a = CE::Class(ex(&d, "A"));
    let (dp, ep) = (ex(&d, "d"), ex(&d, "e"));
    let ax = logical(&o);
    let integer = DR::Datatype(xsd(&d, "integer"));
    for want in [
        Axiom::SubClassOf(a.clone(), CE::DataSomeValuesFrom(dp, integer.clone())),
        Axiom::SubClassOf(
            a.clone(),
            CE::DataAllValuesFrom(
                dp,
                DR::Restriction(
                    xsd(&d, "integer"),
                    vec![
                        (xsd(&d, "minInclusive"), lit("18", "integer")),
                        (xsd(&d, "maxExclusive"), lit("120", "integer")),
                    ],
                ),
            ),
        ),
        Axiom::SubClassOf(a.clone(), CE::DataHasValue(dp, lit("x", "string"))),
        Axiom::SubClassOf(
            a.clone(),
            CE::DataMaxCardinality(1, ep, DR::Datatype(xsd(&d, "string"))),
        ),
        Axiom::DataPropertyRange(
            ep,
            DR::UnionOf(vec![
                DR::Datatype(xsd(&d, "string")),
                DR::ComplementOf(Box::new(DR::Datatype(xsd(&d, "boolean")))),
            ]),
        ),
        Axiom::DataPropertyRange(
            dp,
            DR::OneOf(vec![lit("1", "integer"), lit("2", "integer")]),
        ),
    ] {
        assert!(ax.contains(&want), "missing {want:?} in {ax:?}");
    }
    assert!(
        check_dl(&d, &o).is_dl(),
        "{:?}",
        check_dl(&d, &o).violations
    );
}

#[test]
fn property_axioms_and_characteristics() {
    let (d, _, o) = ok(&with_decls(
        r#"
        :p rdfs:subPropertyOf :q . :d rdfs:subPropertyOf :e .
        :r owl:propertyChainAxiom ( :p :q ) .
        :p owl:equivalentProperty :q . :p owl:propertyDisjointWith :r .
        :p rdfs:domain :A . :p rdfs:range :B . :d rdfs:domain :A .
        :q a owl:TransitiveProperty, owl:SymmetricProperty, owl:ReflexiveProperty .
        :p a owl:InverseFunctionalProperty, owl:AsymmetricProperty, owl:IrreflexiveProperty .
        [ a owl:AllDisjointProperties ; owl:members ( :d :e ) ] .
        :A owl:hasKey ( :p :d ) .
        :A owl:disjointUnionOf ( :B :C ) .
    "#,
    ));
    let (p, q, r) = (
        OP::Named(ex(&d, "p")),
        OP::Named(ex(&d, "q")),
        OP::Named(ex(&d, "r")),
    );
    let (dp, ep) = (ex(&d, "d"), ex(&d, "e"));
    let (a, b, c) = (
        CE::Class(ex(&d, "A")),
        CE::Class(ex(&d, "B")),
        CE::Class(ex(&d, "C")),
    );
    let ax = logical(&o);
    for want in [
        Axiom::SubObjectPropertyOf(vec![p], q),
        Axiom::SubDataPropertyOf(dp, ep),
        Axiom::SubObjectPropertyOf(vec![p, q], r),
        Axiom::EquivalentObjectProperties(vec![p, q]),
        Axiom::DisjointObjectProperties(vec![p, r]),
        Axiom::ObjectPropertyDomain(p, a.clone()),
        Axiom::ObjectPropertyRange(p, b.clone()),
        Axiom::DataPropertyDomain(dp, a.clone()),
        Axiom::TransitiveObjectProperty(q),
        Axiom::SymmetricObjectProperty(q),
        Axiom::ReflexiveObjectProperty(q),
        Axiom::InverseFunctionalObjectProperty(p),
        Axiom::AsymmetricObjectProperty(p),
        Axiom::IrreflexiveObjectProperty(p),
        Axiom::DisjointDataProperties(vec![dp, ep]),
        Axiom::HasKey(a.clone(), vec![p], vec![dp]),
        Axiom::DisjointUnion(ex(&d, "A"), vec![b, c]),
    ] {
        assert!(ax.contains(&want), "missing {want:?}");
    }
}

#[test]
fn assertions_full_abox() {
    let (d, _, o) = ok(&with_decls(
        r#"
        :a a :A , [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :B ] .
        :a :p :b . :a :d "7"^^xsd:integer . :a :p _:anon . _:anon a :B .
        :a owl:sameAs :c . :a owl:differentFrom :b .
        [ a owl:AllDifferent ; owl:distinctMembers ( :a :b :c ) ] .
        [ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ; owl:assertionProperty :p ; owl:targetIndividual :c ] .
        [ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ; owl:assertionProperty :d ; owl:targetValue 8 ] .
        :b a owl:NamedIndividual .
    "#,
    ));
    let (a, b, c) = (
        Individual::Named(ex(&d, "a")),
        Individual::Named(ex(&d, "b")),
        Individual::Named(ex(&d, "c")),
    );
    let p = OP::Named(ex(&d, "p"));
    let dp = ex(&d, "d");
    let ax = logical(&o);
    for want in [
        Axiom::ClassAssertion(CE::Class(ex(&d, "A")), a),
        Axiom::ClassAssertion(CE::SomeValuesFrom(p, Box::new(CE::Class(ex(&d, "B")))), a),
        Axiom::ObjectPropertyAssertion(p, a, b),
        Axiom::DataPropertyAssertion(dp, a, lit("7", "integer")),
        Axiom::SameIndividual(vec![a, c]),
        Axiom::DifferentIndividuals(vec![a, b]),
        Axiom::DifferentIndividuals(vec![a, b, c]),
        Axiom::NegativeObjectPropertyAssertion(p, a, c),
        Axiom::NegativeDataPropertyAssertion(dp, a, lit("8", "integer")),
    ] {
        assert!(ax.contains(&want), "missing {want:?}");
    }
    assert!(ax.iter().any(|x| matches!(
        x,
        Axiom::ObjectPropertyAssertion(_, _, Individual::Anonymous(_))
    )));
    assert!(o
        .axioms
        .iter()
        .any(|r| r.axiom == Axiom::Declaration(EntityKind::NamedIndividual, ex(&d, "b"))));
}

#[test]
fn annotations_headers_and_reification_carry_no_logic() {
    let (d, _, o) = ok(&with_decls(
        r#"
        <http://ex/onto> a owl:Ontology ; rdfs:label "onto" ; owl:versionInfo "1" ; owl:imports <http://ex/other> .
        :note a owl:AnnotationProperty .
        :A rdfs:label "A" ; rdfs:comment "a class" ; :note "hello" .
        :A rdfs:subClassOf :B .
        [ a owl:Axiom ; owl:annotatedSource :A ; owl:annotatedProperty rdfs:subClassOf ;
          owl:annotatedTarget :B ; rdfs:comment "why" ] .
    "#,
    ));
    assert_eq!(
        logical(&o),
        vec![Axiom::SubClassOf(
            CE::Class(ex(&d, "A")),
            CE::Class(ex(&d, "B"))
        )]
    );
    assert_eq!(o.imports, vec![ex(&d, "other")]);
}

#[test]
fn owl1_named_class_definitions_and_property_inference() {
    // No declarations at all: the kinds come from unambiguous uses.
    let (d, _, o) = ok(r#"
        :A owl:intersectionOf ( :B [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :C ] ) .
        :B rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :d ; owl:someValuesFrom xsd:string ] .
        :C rdfs:subClassOf owl:Thing .
        :x :p :y .
    "#);
    let ax = logical(&o);
    let p = OP::Named(ex(&d, "p"));
    assert!(ax.contains(&Axiom::EquivalentClasses(vec![
        CE::Class(ex(&d, "A")),
        CE::IntersectionOf(vec![
            CE::Class(ex(&d, "B")),
            CE::SomeValuesFrom(p, Box::new(CE::Class(ex(&d, "C"))))
        ])
    ])));
    assert!(ax.contains(&Axiom::ObjectPropertyAssertion(
        p,
        Individual::Named(ex(&d, "x")),
        Individual::Named(ex(&d, "y"))
    )));
    let report = check_dl(&d, &o);
    assert!(report
        .violations
        .iter()
        .any(|v| v.kind == ViolationKind::Undeclared));
}

#[test]
fn undeclared_assertion_only_predicates_are_annotations_with_a_note() {
    let (d, _, o) = ok(r#"
        :A a owl:Class . :A <http://purl.org/dc/terms/creator> "Jo" .
    "#);
    assert!(logical(&o).is_empty());
    assert_eq!(o.notes.len(), 1);
    let report = check_dl(&d, &o);
    assert!(report
        .violations
        .iter()
        .any(|v| v.kind == ViolationKind::ExtractionNote));
    assert!(!report.violations.iter().any(|v| v.blocks_reasoning()));
}

#[test]
fn provenance_covers_the_blank_node_structure() {
    let (_, t, o) = ok(&with_decls(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom [ owl:unionOf ( :B :C ) ] ] .",
    ));
    let rec = o.logical_axioms().next().expect("one axiom");
    // subClassOf + restriction typing + onProperty + someValuesFrom + unionOf + 2 list cells × 2
    assert_eq!(rec.source.len(), 9, "{:?}", rec.source);
    let declarations = o.axioms.iter().filter(|r| !r.axiom.is_logical()).count();
    assert_eq!(declarations + rec.source.len(), t.len());
}

#[test]
fn duplicate_triples_are_read_once() {
    let (d, mut t) = parse(&with_decls(":A rdfs:subClassOf :B ."));
    let dup = *t.last().unwrap();
    t.push(dup);
    let o = extract(&d, &t).unwrap();
    assert_eq!(logical(&o).len(), 1);
}

// ============================== REJECT ==============================

#[test]
fn rejects_an_orphan_restriction() {
    let e = err(&with_decls(
        "[ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :B ] .",
    ));
    assert_eq!(e.kind, ExtractErrorKind::Unparsed);
    assert!(!e.triples.is_empty());
}

#[test]
fn rejects_an_orphan_list() {
    let e = err(&with_decls("_:l rdf:first :A ; rdf:rest rdf:nil ."));
    assert_eq!(e.kind, ExtractErrorKind::Unparsed);
}

#[test]
fn rejects_a_stray_triple_on_a_structural_node() {
    let e = err(&with_decls(
        ":A rdfs:subClassOf _:r . _:r a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :B ; :p :A .",
    ));
    // The stray triple is read as an assertion about the restriction node and refused.
    assert!(matches!(
        e.kind,
        ExtractErrorKind::Unparsed | ExtractErrorKind::Malformed
    ));
    assert!(!e.triples.is_empty());
}

#[test]
fn rejects_two_fillers() {
    let e = err(&with_decls(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :B ; owl:allValuesFrom :C ] .",
    ));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
}

#[test]
fn rejects_cyclic_and_branching_lists() {
    let e = err(&with_decls(
        ":A owl:equivalentClass [ owl:unionOf _:l ] . _:l rdf:first :B ; rdf:rest _:l .",
    ));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
    let e = err(&with_decls(
        ":A owl:equivalentClass [ owl:unionOf _:l ] . _:l rdf:first :B, :C ; rdf:rest rdf:nil .",
    ));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
}

#[test]
fn rejects_property_punning_and_unsettled_kinds() {
    let e = err(":p a owl:ObjectProperty, owl:DatatypeProperty . :a :p :b .");
    assert_eq!(e.kind, ExtractErrorKind::Ambiguous);
    // A cardinality alone cannot tell an object from a data property.
    let e = err(":A a owl:Class . :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:maxCardinality 1 ] .");
    assert_eq!(e.kind, ExtractErrorKind::Ambiguous);
}

#[test]
fn rejects_n_ary_data_restrictions_and_bad_cardinalities() {
    let e = err(&with_decls(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperties ( :d :e ) ; owl:someValuesFrom xsd:integer ] .",
    ));
    assert_eq!(e.kind, ExtractErrorKind::Unsupported);
    let e = err(&with_decls(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:maxCardinality -1 ] .",
    ));
    assert_eq!(e.kind, ExtractErrorKind::Unsupported);
    let e = err(&with_decls(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:maxCardinality \"a\" ] .",
    ));
    assert_eq!(e.kind, ExtractErrorKind::Unsupported);
}

#[test]
fn rejects_a_datatype_in_a_class_position_and_a_literal_individual() {
    let e = err(&with_decls(":A rdfs:subClassOf xsd:string ."));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
    let e = err(&with_decls(":a :p \"lit\" ."));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
}

#[test]
fn rejects_unknown_reserved_vocabulary_triples() {
    let e = err(&with_decls(":A owl:fooBar :B ."));
    assert_eq!(e.kind, ExtractErrorKind::Unparsed);
}

// ============================== PROFILE ==============================

fn violations(body: &str) -> Vec<ViolationKind> {
    let (d, _, o) = ok(body);
    check_dl(&d, &o)
        .violations
        .into_iter()
        .map(|v| v.kind)
        .collect()
}

#[test]
fn profile_non_simple_property_under_cardinality() {
    let v = violations(&with_decls(
        ":p a owl:TransitiveProperty . :q rdfs:subPropertyOf :p .
         :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:maxCardinality 1 ] .
         :q a owl:FunctionalProperty .",
    ));
    assert_eq!(
        v.iter()
            .filter(|k| **k == ViolationKind::NonSimpleProperty)
            .count(),
        1
    );
    // :q is a sub-property of a transitive property but is itself simple.
    let v = violations(&with_decls(
        ":p a owl:TransitiveProperty . :p rdfs:subPropertyOf :q . :q a owl:FunctionalProperty .",
    ));
    assert_eq!(v, vec![ViolationKind::NonSimpleProperty]);
}

#[test]
fn profile_regular_and_irregular_chains() {
    // Regular: R∘S ⊑ R with S < R; transitivity form.
    assert!(violations(&with_decls(
        ":p owl:propertyChainAxiom ( :p :q ) . :r owl:propertyChainAxiom ( :r :r ) ."
    ))
    .is_empty());
    // Irregular: p∘q ⊑ r and r∘p ⊑ q needs q < r and r < q.
    let v = violations(&with_decls(
        ":r owl:propertyChainAxiom ( :p :q ) . :q owl:propertyChainAxiom ( :r :p ) .",
    ));
    assert!(
        v.contains(&ViolationKind::IrregularPropertyHierarchy),
        "{v:?}"
    );
    // Irregular: R∘S∘R ⊑ R.
    let v = violations(&with_decls(":r owl:propertyChainAxiom ( :r :q :r ) ."));
    assert!(
        v.contains(&ViolationKind::IrregularPropertyHierarchy),
        "{v:?}"
    );
}

#[test]
fn profile_punning_and_reserved_vocabulary() {
    let v = violations(":C a owl:Class, rdfs:Datatype .");
    assert!(v.contains(&ViolationKind::IllegalPunning), "{v:?}");
    // Class / individual punning is allowed.
    assert!(violations(":C a owl:Class . :C a :C .").is_empty());
}

#[test]
fn profile_literals_facets_and_datatypes() {
    let v = violations(&with_decls(":a :d \"sixteen\"^^xsd:nonNegativeInteger ."));
    assert_eq!(v, vec![ViolationKind::IllTypedLiteral]);
    let v = violations(&with_decls(
        ":d rdfs:range [ a rdfs:Datatype ; owl:onDatatype xsd:boolean ; owl:withRestrictions ( [ xsd:minLength 1 ] ) ] .",
    ));
    assert_eq!(v, vec![ViolationKind::InvalidFacet]);
    let v = violations(&with_decls(":d rdfs:range xsd:date ."));
    assert_eq!(v, vec![ViolationKind::UnknownDatatype]);
    let v = violations(&with_decls(
        ":dt a rdfs:Datatype . :dt owl:equivalentClass [ a rdfs:Datatype ; owl:datatypeComplementOf :dt ] .",
    ));
    assert_eq!(v, vec![ViolationKind::InvalidDatatypeDefinition]);
    assert!(violations(&with_decls(
        ":adult a rdfs:Datatype . :adult owl:equivalentClass [ a rdfs:Datatype ; owl:onDatatype xsd:integer ; owl:withRestrictions ( [ xsd:minInclusive 18 ] ) ] . :d rdfs:range :adult ."
    ))
    .is_empty());
}

#[test]
fn profile_anonymous_individual_cycles() {
    let v = violations(&with_decls("_:x :p _:y . _:y :q _:z . _:z :r _:x ."));
    assert_eq!(v, vec![ViolationKind::AnonymousIndividualCycle]);
    assert!(violations(&with_decls("_:x :p _:y . _:y :q _:z . :a :r _:x .")).is_empty());
}

#[test]
fn profile_violations_name_their_triples() {
    let (d, t) = parse(&with_decls(":a :d \"x\"^^xsd:integer ."));
    let o = extract(&d, &t).unwrap();
    let r = check_dl(&d, &o);
    let v = &r.violations[0];
    assert_eq!(v.source.len(), 1);
    let [s, _, _] = t[v.source[0]];
    assert_eq!(s, ex(&d, "a"));
}

// ============================== REVIEW REGRESSIONS ==============================

#[test]
fn gci_with_an_anonymous_subclass() {
    let (d, _, o) = ok(&with_decls(
        "[ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :B ] rdfs:subClassOf :A .
         [ owl:unionOf ( :A :B ) ] owl:equivalentClass :C .
         [ owl:complementOf :A ] owl:disjointWith [ owl:complementOf :B ] .",
    ));
    let (a, b, c, p) = (ex(&d, "A"), ex(&d, "B"), ex(&d, "C"), ex(&d, "p"));
    let ax = logical(&o);
    assert!(ax.contains(&Axiom::SubClassOf(
        CE::SomeValuesFrom(OP::Named(p), Box::new(CE::Class(b))),
        CE::Class(a)
    )));
    assert!(ax.contains(&Axiom::EquivalentClasses(vec![
        CE::UnionOf(vec![CE::Class(a), CE::Class(b)]),
        CE::Class(c)
    ])));
    assert!(ax.iter().any(|x| matches!(x, Axiom::DisjointClasses(_))));
    assert!(check_dl(&d, &o).is_dl());
}

#[test]
fn n_ary_and_negative_nodes_in_any_triple_order() {
    // Typing triples last, as a sorted N-Triples dump or a store scan may give them.
    let body = "@prefix : <http://ex/> .
        @prefix owl: <http://www.w3.org/2002/07/owl#> .
        @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
        :a a owl:NamedIndividual . :b a owl:NamedIndividual . :p a owl:ObjectProperty .
        _:x owl:distinctMembers ( :a :b ) .
        _:n owl:sourceIndividual :a . _:n owl:assertionProperty :p . _:n owl:targetIndividual :b .
        _:x rdf:type owl:AllDifferent .
        _:n rdf:type owl:NegativePropertyAssertion .";
    let (d, t) = Graph::parse_to_triples(body, "turtle").expect("parse");
    let o = extract(&d, &t).unwrap_or_else(|e| panic!("expected accept, got {e}"));
    let ax = logical(&o);
    let (a, b, p) = (ex(&d, "a"), ex(&d, "b"), ex(&d, "p"));
    assert!(ax.contains(&Axiom::DifferentIndividuals(vec![
        Individual::Named(a),
        Individual::Named(b)
    ])));
    assert!(ax.contains(&Axiom::NegativeObjectPropertyAssertion(
        OP::Named(p),
        Individual::Named(a),
        Individual::Named(b)
    )));
}

#[test]
fn axioms_about_an_inverse_property_expression() {
    let (d, _, o) = ok(&with_decls(
        "_:x owl:inverseOf :p . _:x rdfs:subPropertyOf :q . _:x a owl:TransitiveProperty .",
    ));
    let (p, q) = (ex(&d, "p"), ex(&d, "q"));
    let ax = logical(&o);
    assert!(ax.contains(&Axiom::SubObjectPropertyOf(
        vec![OP::Inverse(p)],
        OP::Named(q)
    )));
    assert!(ax.contains(&Axiom::TransitiveObjectProperty(OP::Inverse(p))));
}

#[test]
fn unqualified_data_cardinality_is_over_rdfs_literal() {
    let (d, _, o) = ok(&with_decls(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :d ; owl:minCardinality 1 ] .",
    ));
    assert!(logical(&o).contains(&Axiom::SubClassOf(
        CE::Class(ex(&d, "A")),
        CE::DataMinCardinality(1, ex(&d, "d"), DR::TopLiteral)
    )));
    assert!(check_dl(&d, &o).is_dl());
}

#[test]
fn huge_years_are_parsed_without_overflow() {
    use sparq_reason_dl::owl2dl::datatypes::parse_value;
    let dt = format!("{XSD}dateTime");
    for lex in [
        "999999999999999999-01-01T00:00:00",
        "-999999999999999999-12-31T23:59:59Z",
    ] {
        // Valid, or refused as unsupported; never a panic.
        let _ = parse_value(lex, &dt, None);
    }
}

#[test]
fn a_long_chain_of_anonymous_individuals_is_checked_iteratively() {
    let mut body = with_decls("");
    for i in 0..50_000 {
        body.push_str(&format!("_:x{i} :p _:x{} .\n", i + 1));
    }
    let v = violations(&body);
    assert!(!v.contains(&ViolationKind::AnonymousIndividualCycle));
}

// ============================== CROSS-MODEL REVIEW REGRESSIONS ==============================

fn report(body: &str) -> sparq_reason_dl::owl2dl::DlProfileReport {
    let (d, _, o) = ok(body);
    check_dl(&d, &o)
}

#[test]
fn ontology_header_keeps_logical_triples() {
    let (d, _, o) = ok(&with_decls(
        "<http://ex/o> a owl:Ontology , :A ; rdfs:comment \"about\" ; owl:versionIRI <http://ex/o/1> .",
    ));
    let onto = ex(&d, "o");
    assert!(logical(&o).contains(&Axiom::ClassAssertion(
        CE::Class(ex(&d, "A")),
        Individual::Named(onto)
    )));
    assert_eq!(o.annotations.len(), 1);
    assert!(matches!(
        o.annotations[0].target,
        sparq_reason_dl::owl2dl::model::AnnotationTarget::Ontology(id) if id == onto
    ));
}

#[test]
fn reifications_are_validated_and_kept() {
    let body = ":A rdfs:subClassOf :B .
        [ a owl:Axiom ; owl:annotatedSource :A ; owl:annotatedProperty rdfs:subClassOf ;
          owl:annotatedTarget :B ; rdfs:comment \"why\" ] .";
    let (d, t, o) = ok(&with_decls(body));
    assert_eq!(o.annotations.len(), 1);
    let sparq_reason_dl::owl2dl::model::AnnotationTarget::Axiom(main) = o.annotations[0].target
    else {
        panic!("axiom annotation expected");
    };
    let sub = iri(&d, "http://www.w3.org/2000/01/rdf-schema#subClassOf");
    assert_eq!(t[main], [ex(&d, "A"), sub, ex(&d, "B")]);
    // A reification of a triple that is not in the graph is refused.
    let e = err(&with_decls(
        "[ a owl:Axiom ; owl:annotatedSource :A ; owl:annotatedProperty rdfs:subClassOf ;
          owl:annotatedTarget :C ; rdfs:comment \"why\" ] .",
    ));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
    // A logical triple on a reification node is refused, not dropped.
    let e = err(&with_decls(
        ":A rdfs:subClassOf :B .
         _:x a owl:Axiom ; owl:annotatedSource :A ; owl:annotatedProperty rdfs:subClassOf ;
          owl:annotatedTarget :B ; rdfs:subClassOf :C .",
    ));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
}

#[test]
fn annotations_are_kept_and_checked() {
    let (d, _, o) = ok(&with_decls(
        ":A rdfs:label \"bad\"^^xsd:integer . :ap a owl:AnnotationProperty . :ap rdfs:subPropertyOf :undeclaredAp .",
    ));
    let ax = logical(&o);
    assert!(ax.is_empty(), "annotation axioms are not logical: {ax:?}");
    assert!(o.axioms.iter().any(|r| matches!(
        r.axiom,
        Axiom::SubAnnotationPropertyOf(a, b) if a == ex(&d, "ap") && b == ex(&d, "undeclaredAp")
    )));
    let kinds: Vec<ViolationKind> = check_dl(&d, &o).violations.iter().map(|v| v.kind).collect();
    assert!(kinds.contains(&ViolationKind::IllTypedLiteral));
    assert!(kinds.contains(&ViolationKind::Undeclared));
}

#[test]
fn declaring_reserved_vocabulary_is_a_violation() {
    let v = violations(&with_decls(
        "owl:madeUp a owl:Class . xsd:integer a owl:Class .",
    ));
    assert_eq!(
        v.iter()
            .filter(|k| **k == ViolationKind::ReservedVocabulary)
            .count(),
        2
    );
}

#[test]
fn bottom_object_property_is_not_simple() {
    let v = violations(&with_decls(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty owl:bottomObjectProperty ; owl:maxCardinality 1 ] .",
    ));
    assert!(v.contains(&ViolationKind::NonSimpleProperty));
}

#[test]
fn equivalent_properties_cannot_hide_an_irregular_chain() {
    let v = violations(&with_decls(
        ":p owl:equivalentProperty :q . :p owl:propertyChainAxiom ( :q :r ) .",
    ));
    assert!(v.contains(&ViolationKind::IrregularPropertyHierarchy));
    // A regular chain over the same hierarchy stays legal.
    let v = violations(&with_decls(
        ":p owl:equivalentProperty :q . :p owl:propertyChainAxiom ( :r :r ) .",
    ));
    assert!(!v.contains(&ViolationKind::IrregularPropertyHierarchy));
}

#[test]
fn user_datatypes_need_exactly_one_definition() {
    let decl = ":dt a rdfs:Datatype . :d rdfs:range :dt .";
    assert!(violations(&with_decls(decl)).contains(&ViolationKind::UnknownDatatype));
    assert!(violations(&with_decls(&format!(
        "{decl} :dt owl:equivalentClass xsd:integer ."
    )))
    .is_empty());
    let v = violations(&with_decls(&format!(
        "{decl} :dt owl:equivalentClass xsd:integer . :dt owl:equivalentClass xsd:string ."
    )));
    assert!(v.contains(&ViolationKind::InvalidDatatypeDefinition));
    // A literal of a user datatype has no value.
    let v = violations(&with_decls(
        ":dt a rdfs:Datatype . :dt owl:equivalentClass xsd:integer . :x :d \"1\"^^:dt .",
    ));
    assert!(v.contains(&ViolationKind::UnknownDatatype));
}

#[test]
fn unchecked_restrictions_make_the_verdict_unknown() {
    let pattern = ":d rdfs:range [ a rdfs:Datatype ; owl:onDatatype xsd:string ; owl:withRestrictions ( [ xsd:pattern \"[a-z]+\" ] ) ] .";
    let r = report(&with_decls(pattern));
    assert_eq!(r.verdict(), None);
    assert!(!r.is_dl());
    let r = report(&with_decls(
        "<http://ex/o> a owl:Ontology ; owl:imports <http://ex/other> .",
    ));
    assert_eq!(r.verdict(), None);
    let r = report(&with_decls(":a :p _:x ."));
    assert_eq!(r.verdict(), None);
    let r = report(&with_decls("_:x :p _:y ."));
    assert_eq!(r.verdict(), Some(true));
}

#[test]
fn language_range_facets_are_validated() {
    let range = |r: &str| {
        format!(
            ":d rdfs:range [ a rdfs:Datatype ; owl:onDatatype rdf:PlainLiteral ; owl:withRestrictions ( [ rdf:langRange \"{r}\" ] ) ] ."
        )
    };
    assert!(violations(&with_decls(&range("en-gb"))).is_empty());
    assert!(violations(&with_decls(&range("*"))).is_empty());
    // Extended ranges are not basic ranges (RFC 4647 §2.1).
    assert!(violations(&with_decls(&range("en-*"))).contains(&ViolationKind::InvalidFacet));
    assert!(violations(&with_decls(&range("en--gb"))).contains(&ViolationKind::InvalidFacet));
}

#[test]
fn characteristics_only_object_properties_have_declare_one() {
    let (d, _, o) = ok(":p a owl:TransitiveProperty .");
    assert!(o
        .axioms
        .iter()
        .any(|r| r.axiom == Axiom::Declaration(EntityKind::ObjectProperty, ex(&d, "p"))));
    assert!(check_dl(&d, &o).is_dl());
}

#[test]
fn n_ary_axioms_need_their_operands() {
    let e = err(&with_decls(":A owl:disjointUnionOf ( :B ) ."));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
    let e = err(&with_decls(
        "[ a owl:AllDisjointClasses ; owl:members ( :A ) ] .",
    ));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
    let v = violations(&with_decls(":p owl:equivalentProperty :p ."));
    assert!(v.contains(&ViolationKind::InsufficientOperands));
}

#[test]
fn empty_anonymous_booleans_follow_table_15() {
    let (d, _, o) = ok(&with_decls(
        ":A rdfs:subClassOf [ owl:intersectionOf () ] . :B rdfs:subClassOf [ owl:unionOf () ] .",
    ));
    let ax = logical(&o);
    assert!(ax.contains(&Axiom::SubClassOf(CE::Class(ex(&d, "A")), CE::Thing)));
    assert!(ax.contains(&Axiom::SubClassOf(CE::Class(ex(&d, "B")), CE::Nothing)));
}

#[test]
fn a_node_is_a_class_or_a_data_range_not_both() {
    let e = err(&with_decls(
        "_:x a owl:Class , rdfs:Datatype ; owl:complementOf :A ; owl:datatypeComplementOf xsd:string .
         :B rdfs:subClassOf _:x . :d rdfs:range _:x .",
    ));
    assert!(matches!(
        e.kind,
        ExtractErrorKind::Ambiguous | ExtractErrorKind::Malformed
    ));
}

#[test]
fn rational_order_is_exact() {
    use sparq_reason_dl::owl2dl::datatypes::{parse_value, Value};
    let dec = format!("{XSD}decimal");
    let v = |s: &str| match parse_value(s, &dec, None).unwrap() {
        Value::Real(r) => r,
        other => panic!("{other:?}"),
    };
    let (a, b) = (v("0.50000000000000000001"), v("0.50000000000000000002"));
    assert!(a < b);
    assert!(v("-0.50000000000000000002") < v("-0.50000000000000000001"));
}

#[test]
fn base64_and_xml_names_follow_their_grammars() {
    use sparq_reason_dl::owl2dl::datatypes::parse_value;
    let b64 = format!("{XSD}base64Binary");
    assert!(parse_value("QQ==", &b64, None).is_ok());
    assert!(parse_value("QR==", &b64, None).is_err());
    assert!(parse_value("QUI=", &b64, None).is_ok());
    assert!(parse_value("QUJ=", &b64, None).is_err());
    let nc = format!("{XSD}NCName");
    // U+0301 (a combining accent) is a NameChar but not alphabetic.
    assert!(parse_value("e\u{301}", &nc, None).is_ok());
    assert!(parse_value("\u{301}e", &nc, None).is_err());
}

#[test]
fn only_three_characteristics_declare_an_object_property() {
    let base = ":a a owl:NamedIndividual . :b a owl:NamedIndividual . :a :p :b .";
    for ch in [
        "AsymmetricProperty",
        "ReflexiveProperty",
        "IrreflexiveProperty",
    ] {
        let v = violations(&format!("{base} :p a owl:{ch} ."));
        assert!(v.contains(&ViolationKind::Undeclared), "{ch}: {v:?}");
    }
    for ty in ["rdfs:Class", "owl:DeprecatedClass"] {
        let v = violations(&format!(":A a {ty} . :x a owl:NamedIndividual , :A ."));
        assert!(v.contains(&ViolationKind::Undeclared), "{ty}: {v:?}");
    }
}

#[test]
fn annotations_on_axiom_nodes_are_kept_and_checked() {
    let body = ":a a owl:NamedIndividual . :b a owl:NamedIndividual .
        [] a owl:AllDifferent ; owl:members ( :a :b ) ; rdfs:comment \"abc\"^^xsd:integer .";
    let (d, _, o) = ok(body);
    assert_eq!(o.annotations.len(), 1);
    assert!(matches!(
        o.annotations[0].target,
        AnnotationTarget::Axiom(_)
    ));
    assert!(!check_dl(&d, &o).is_dl());
}

#[test]
fn an_ontology_annotation_can_be_annotated() {
    let (_, _, o) = ok("<http://ex/o> a owl:Ontology ; rdfs:comment \"c\" .
        [] a owl:Annotation ; owl:annotatedSource <http://ex/o> ; owl:annotatedProperty rdfs:comment ;
           owl:annotatedTarget \"c\" ; rdfs:label \"x\" .");
    assert_eq!(o.annotations.len(), 2);
    assert_eq!(o.annotations[1].target, AnnotationTarget::Annotation(0));
}

#[test]
fn annotation_provenance_uses_input_indices() {
    let (d, mut t) = parse("<http://ex/o> a owl:Ontology ; rdfs:comment \"x\"^^xsd:integer .");
    t.insert(0, t[0]);
    let o = extract(&d, &t).unwrap();
    let i = t
        .iter()
        .rposition(|x| x[1] == iri(&d, "http://www.w3.org/2000/01/rdf-schema#comment"))
        .unwrap();
    assert_eq!(o.annotations[0].source, vec![i]);
}

#[test]
fn annotating_an_assertion_does_not_change_how_it_is_read() {
    let plain = ":a a owl:NamedIndividual . :a :note \"x\" .";
    let annotated = format!(
        "{plain} [] a owl:Axiom ; owl:annotatedSource :a ; owl:annotatedProperty :note ;
            owl:annotatedTarget \"x\" ; rdfs:comment \"c\" ."
    );
    let (_, _, a) = ok(plain);
    let (_, _, b) = ok(&annotated);
    assert_eq!(logical(&a), logical(&b));
}

#[test]
fn base64_whitespace_is_xml_whitespace_only() {
    use sparq_reason_dl::owl2dl::datatypes::parse_value;
    let b64 = format!("{XSD}base64Binary");
    assert!(parse_value("aG k=", &b64, None).is_ok());
    assert!(parse_value("aGk=\u{B}", &b64, None).is_err());
    assert!(parse_value("aG\u{A0}k=", &b64, None).is_err());
}

#[test]
fn float_literals_round_once() {
    use sparq_reason_dl::owl2dl::datatypes::parse_value;
    let f = format!("{XSD}float");
    let a = parse_value(
        "1.000000059604644776257986737988403547205962240695953369140625",
        &f,
        None,
    )
    .unwrap();
    assert_ne!(a, parse_value("1.0", &f, None).unwrap());
}

#[test]
fn unshared_expressions_are_not_charged() {
    // Deep unshared nesting: charging each first decode for its whole subtree would cost
    // quadratically in the depth and pass the expansion cap.
    let depth = 60;
    let mut s = String::from(":r a owl:ObjectProperty . :B a owl:Class .\n");
    for a in 0..1200 {
        let mut e = ":B".to_string();
        for _ in 0..depth {
            e = format!("[ a owl:Restriction ; owl:onProperty :r ; owl:someValuesFrom {e} ]");
        }
        s.push_str(&format!(":A{a} a owl:Class ; rdfs:subClassOf {e} .\n"));
    }
    ok(&s);
}

// ===================== third review round: one test per finding =====================

fn verdict(body: &str) -> Option<bool> {
    let (d, _, o) = ok(body);
    check_dl(&d, &o).verdict()
}

#[test]
fn unrepresented_literal_values_are_unchecked_and_garbage_is_still_ill_typed() {
    // A valid dateTime whose year is beyond i64: not represented, so no verdict.
    let big = with_decls(":a :d \"99999999999999999999-01-01T00:00:00Z\"^^xsd:dateTime .");
    assert_eq!(verdict(&big), None);
    // The same year with an impossible month, day and time is ill-typed.
    let bad = with_decls(":a :d \"99999999999999999999-13-45T99:99:99\"^^xsd:dateTime .");
    assert_eq!(violations(&bad), vec![ViolationKind::IllTypedLiteral]);
    // A value-space bound beyond the represented range is unchecked, not a bad facet.
    let facet = with_decls(
        ":d rdfs:range [ a rdfs:Datatype ; owl:onDatatype xsd:integer ; owl:withRestrictions ( [ xsd:minInclusive \"-170141183460469231731687303715884105728\"^^xsd:integer ] ) ] .",
    );
    assert_eq!(verdict(&facet), None);
}

#[test]
fn a_reserved_iri_cannot_be_defined_as_a_datatype() {
    let v = violations(&with_decls(
        "xsd:myType a rdfs:Datatype ; owl:equivalentClass xsd:integer .",
    ));
    assert!(v.contains(&ViolationKind::ReservedVocabulary), "{v:?}");
    // Using a reserved datatype outside the map is reported once, as unknown.
    let v = violations(&with_decls(":d rdfs:range xsd:date ."));
    assert_eq!(v, vec![ViolationKind::UnknownDatatype]);
}

#[test]
fn a_declared_only_datatype_needs_a_definition() {
    let v = violations(":dt a rdfs:Datatype .");
    assert_eq!(v, vec![ViolationKind::UnknownDatatype]);
    let v = violations(":dt a rdfs:Datatype ; owl:equivalentClass xsd:integer .");
    assert!(v.is_empty(), "{v:?}");
}

#[test]
fn ontology_headers_are_validated() {
    for body in [
        "<http://ex/o1> a owl:Ontology . <http://ex/o2> a owl:Ontology .",
        "<http://ex/o> a owl:Ontology ; owl:versionIRI <http://ex/v1>, <http://ex/v2> .",
        "[] a owl:Ontology ; owl:versionIRI <http://ex/v1> .",
        "<http://ex/o> a owl:Ontology ; owl:versionIRI \"1\" .",
        "<http://ex/o> a owl:Ontology ; owl:imports \"x\" .",
    ] {
        assert_eq!(err(body).kind, ExtractErrorKind::Malformed, "{body}");
    }
    let (d, _, o) = ok("<http://ex/o> a owl:Ontology ; owl:versionIRI <http://ex/v1> .");
    assert_eq!(o.iri, Some(ex(&d, "o")));
    assert_eq!(o.version_iri, Some(ex(&d, "v1")));
    let v = violations("owl:Thing a owl:Ontology .");
    assert_eq!(v, vec![ViolationKind::ReservedVocabulary]);
}

#[test]
fn rejects_cardinalities_not_written_as_non_negative_integers() {
    for c in [
        "\"0\"^^xsd:positiveInteger",
        "\"40000\"^^xsd:short",
        "\"2\"^^xsd:byte",
        "\"2.0\"^^xsd:decimal",
        "\"-1\"^^xsd:integer",
        "\"x\"^^xsd:nonNegativeInteger",
    ] {
        let e = err(&with_decls(&format!(
            ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:maxCardinality {c} ] ."
        )));
        assert_eq!(e.kind, ExtractErrorKind::Unsupported, "{c}");
    }
    for c in ["2", "\" 2 \"^^xsd:nonNegativeInteger"] {
        ok(&with_decls(&format!(
            ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:maxCardinality {c} ] ."
        )));
    }
}

#[test]
fn an_axiom_reification_must_name_an_axiom() {
    let e = err(&with_decls(
        r#"
        :A owl:equivalentClass [ owl:unionOf _:l ] .
        _:l rdf:first :B ; rdf:rest ( :C ) .
        [ a owl:Axiom ; owl:annotatedSource _:l ; owl:annotatedProperty rdf:first ;
          owl:annotatedTarget :B ; rdfs:comment "x" ] .
    "#,
    ));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
}

#[test]
fn annotation_assertions_and_n_ary_annotations_can_be_annotated() {
    let (_, t, o) = ok(r#"
        :A a owl:Class ; rdfs:comment "c" .
        [ a owl:Annotation ; owl:annotatedSource :A ; owl:annotatedProperty rdfs:comment ;
          owl:annotatedTarget "c" ; rdfs:label "l" ] .
    "#);
    assert!(
        o.annotations
            .iter()
            .any(|a| matches!(a.target, AnnotationTarget::Axiom(j) if j < t.len())),
        "{:?}",
        o.annotations
    );
    ok(r#"
        :a a owl:NamedIndividual . :b a owl:NamedIndividual .
        _:x a owl:AllDifferent ; owl:members ( :a :b ) ; rdfs:comment "c" .
        [ a owl:Annotation ; owl:annotatedSource _:x ; owl:annotatedProperty rdfs:comment ;
          owl:annotatedTarget "c" ; rdfs:label "l" ] .
    "#);
}

#[test]
fn rejects_lists_that_share_a_tail() {
    let e = err(&with_decls(
        r#"
        :A owl:equivalentClass [ owl:unionOf _:l1 ] .
        :B owl:equivalentClass [ owl:unionOf _:l2 ] .
        _:l1 rdf:first :B ; rdf:rest _:t .
        _:l2 rdf:first :A ; rdf:rest _:t .
        _:t rdf:first :C ; rdf:rest rdf:nil .
    "#,
    ));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
    // One list used twice is fine.
    ok(&with_decls(
        ":A owl:equivalentClass [ owl:unionOf _:l ] . :B owl:equivalentClass [ owl:unionOf _:l ] . _:l rdf:first :C ; rdf:rest ( :p ) .",
    ).replace("( :p )", "( :A )"));
}

#[test]
fn deprecated_typings_are_deprecated_annotations() {
    let (d, _, o) = ok(r#"
        :x rdfs:label "uses owl:deprecated" ; owl:deprecated false .
        :A a owl:Class, owl:DeprecatedClass .
        :p a owl:ObjectProperty, owl:DeprecatedProperty .
    "#);
    let dep = iri(&d, "http://www.w3.org/2002/07/owl#deprecated");
    let deprecated: Vec<_> = o
        .axioms
        .iter()
        .filter_map(|r| match &r.axiom {
            Axiom::AnnotationAssertion(p, s, v) if *p == dep => Some((*s, v.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(deprecated.len(), 3, "{deprecated:?}");
    // Without owl:deprecated in the dictionary the typing is kept as a note.
    let (_, _, o) = ok(":A a owl:Class, owl:DeprecatedClass .");
    assert!(o.notes.iter().any(|n| n.message.contains("deprecated")));
}

#[test]
fn assertions_may_use_built_in_properties() {
    let (d, _, o) = ok(":a :q :b . :a owl:topObjectProperty :b . :a owl:bottomObjectProperty :b . :a owl:bottomDataProperty \"x\" .");
    let n = logical(&o)
        .iter()
        .filter(|a| {
            matches!(a, Axiom::ObjectPropertyAssertion(OP::Named(p), _, _) | Axiom::DataPropertyAssertion(p, _, _)
                if iri_of_builtin(&d, *p))
        })
        .count();
    assert_eq!(n, 3);
    assert_eq!(
        err(":a owl:topDataProperty \"x\" .").kind,
        ExtractErrorKind::Unparsed
    );
}

fn iri_of_builtin(d: &Dict, p: Id) -> bool {
    [
        "topObjectProperty",
        "bottomObjectProperty",
        "bottomDataProperty",
    ]
    .iter()
    .any(|l| iri(d, &format!("http://www.w3.org/2002/07/owl#{l}")) == p)
}

#[test]
fn rejects_singleton_data_unions_and_intersections() {
    for c in ["unionOf", "intersectionOf"] {
        let e = err(&with_decls(&format!(
            ":d rdfs:range [ a rdfs:Datatype ; owl:{c} ( xsd:integer ) ] ."
        )));
        assert_eq!(e.kind, ExtractErrorKind::Malformed, "{c}");
    }
}

#[test]
fn operand_distinctness_ignores_set_order() {
    let v = violations(&with_decls(
        "[ owl:unionOf ( :A :B ) ] owl:equivalentClass [ owl:unionOf ( :B :A ) ] .",
    ));
    assert_eq!(v, vec![ViolationKind::InsufficientOperands]);
}

#[test]
fn shared_lists_are_charged_to_the_expansion_budget() {
    // n expressions share one n-item list: about n² copies from O(n) triples.
    let n = 1800;
    let mut body = String::from(DECLS);
    body.push_str("_:L0 rdf:first :A ; rdf:rest _:L1 .\n");
    for i in 1..n {
        body.push_str(&format!("_:L{i} rdf:first :B ; rdf:rest _:L{} .\n", i + 1));
    }
    body.push_str(&format!("_:L{n} rdf:first :C ; rdf:rest rdf:nil .\n"));
    for i in 0..n {
        body.push_str(&format!(
            ":D{i} a owl:Class ; rdfs:subClassOf [ owl:unionOf _:L0 ] .\n"
        ));
    }
    assert_eq!(err(&body).kind, ExtractErrorKind::Unsupported);
}

#[test]
fn literal_lexical_forms_are_strict() {
    for lit in [
        "\"1/ -2\"^^owl:rational",
        "\"1 / 3\"^^owl:rational",
        "\"1/0\"^^owl:rational",
        "\"a\\u0001b\"^^xsd:string",
    ] {
        let v = violations(&with_decls(&format!(":a :d {lit} .")));
        assert_eq!(v, vec![ViolationKind::IllTypedLiteral], "{lit}");
    }
    assert!(violations(&with_decls(":a :d \"1/3\"^^owl:rational .")).is_empty());
    // Plain literal tags: well-formed BCP 47 is checked; a tag of the right shape that is
    // not (a grandfathered or malformed extension tag) leaves the verdict open.
    assert_eq!(
        verdict(&with_decls(":a :d \"x@en-GB\"^^rdf:PlainLiteral .")),
        Some(true)
    );
    for tag in ["i-klingon", "en-a", "en-gb-gb-gb"] {
        let body = with_decls(&format!(":a :d \"x@{tag}\"^^rdf:PlainLiteral ."));
        assert_eq!(verdict(&body), None, "{tag}");
    }
}
