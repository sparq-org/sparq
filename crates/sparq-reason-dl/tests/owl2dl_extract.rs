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
    Axiom, ClassExpression as CE, DataRange as DR, EntityKind, Individual, Literal,
    ObjectProperty as OP,
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
    assert_eq!(e.kind, ExtractErrorKind::Unparsed);
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
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
    let e = err(&with_decls(
        ":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :p ; owl:maxCardinality \"a\" ] .",
    ));
    assert_eq!(e.kind, ExtractErrorKind::Malformed);
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
