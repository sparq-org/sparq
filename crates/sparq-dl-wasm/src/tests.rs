//! Native tests of the JSON-producing core (`check_owl_dl_json`). Each result is parsed back
//! with the crate's own strict JSON parser, so a malformed document fails every test.

use super::check_owl_dl_json;
use super::json::{parse, Value};

const PRE: &str = r#"
@prefix : <http://ex/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

fn run_with(tbox: &str, abox: Option<&str>, options: Option<&str>) -> Value {
    let tbox = format!("{PRE}{tbox}");
    let abox = abox.map(|a| format!("{PRE}{a}"));
    let out = check_owl_dl_json(&tbox, abox.as_deref(), "turtle", options).expect("checked");
    parse(&out).unwrap_or_else(|e| panic!("result is not JSON ({e}): {out}"))
}

fn run(tbox: &str) -> Value {
    run_with(tbox, None, None)
}

fn get<'a>(v: &'a Value, key: &str) -> &'a Value {
    match v {
        Value::Object(m) => m
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("no member {key:?} in {v:?}")),
        _ => panic!("not an object: {v:?}"),
    }
}

fn s(v: &Value) -> &str {
    match v {
        Value::Str(s) => s,
        _ => panic!("not a string: {v:?}"),
    }
}

fn arr(v: &Value) -> &[Value] {
    match v {
        Value::Array(a) => a,
        _ => panic!("not an array: {v:?}"),
    }
}

fn strs(v: &Value) -> Vec<&str> {
    arr(v).iter().map(s).collect()
}

fn pairs(v: &Value) -> Vec<(&str, &str)> {
    arr(v)
        .iter()
        .map(|p| match strs(p).as_slice() {
            [a, b] => (*a, *b),
            other => panic!("not a pair: {other:?}"),
        })
        .collect()
}

/// The member names of the result object, in order: the documented shape.
fn keys(v: &Value) -> Vec<&str> {
    match v {
        Value::Object(m) => m.iter().map(|(k, _)| k.as_str()).collect(),
        _ => panic!("not an object"),
    }
}

const SHAPE: [&str; 6] = [
    "consistent",
    "reason",
    "profile",
    "unsatisfiable",
    "inferredSubClassOf",
    "classification",
];

/// The issue's ALCHIQ example: qualified cardinality, inverse properties, a functional
/// object property, a union domain and `AllDisjointClasses`.
const ALCHIQ: &str = r#"
:Person a owl:Class . :Parent a owl:Class . :Mother a owl:Class . :Woman a owl:Class .
:Man a owl:Class . :Company a owl:Class .
:hasChild a owl:ObjectProperty ; owl:inverseOf :hasParent .
:hasParent a owl:ObjectProperty .
:hasMother a owl:ObjectProperty , owl:FunctionalProperty ; rdfs:subPropertyOf :hasParent .
:worksFor a owl:ObjectProperty ; rdfs:domain [ a owl:Class ; owl:unionOf ( :Man :Woman ) ] .
:Woman rdfs:subClassOf :Person . :Man rdfs:subClassOf :Person .
[] a owl:AllDisjointClasses ; owl:members ( :Man :Woman :Company ) .
:Parent owl:equivalentClass [ a owl:Restriction ; owl:onProperty :hasChild ;
    owl:minQualifiedCardinality "1"^^xsd:nonNegativeInteger ; owl:onClass :Person ] .
:Mother owl:equivalentClass [ a owl:Class ; owl:intersectionOf ( :Woman :Parent ) ] .
"#;

#[test]
fn consistent_alchiq_ontology_with_an_inferred_subsumption() {
    let r = run(ALCHIQ);
    assert_eq!(keys(&r), SHAPE);
    assert_eq!(get(&r, "consistent"), &Value::Bool(true));
    assert_eq!(get(&r, "reason"), &Value::Null);
    let profile = get(&r, "profile");
    assert_eq!(get(profile, "dl"), &Value::Bool(true));
    assert!(arr(get(profile, "violations")).is_empty());
    assert!(arr(get(profile, "unchecked")).is_empty());
    assert_eq!(s(get(&r, "classification")), "complete");
    assert!(arr(get(&r, "unsatisfiable")).is_empty());
    let inferred = pairs(get(&r, "inferredSubClassOf"));
    // Mother ≡ Woman ⊓ Parent and Woman ⊑ Person: Mother ⊑ Person is entailed, not told.
    assert!(inferred.contains(&("http://ex/Mother", "http://ex/Person")));
    // Woman ⊑ Person is asserted, so it is not reported as inferred.
    assert!(!inferred.contains(&("http://ex/Woman", "http://ex/Person")));
    let mut sorted = inferred.clone();
    sorted.sort_unstable();
    assert_eq!(inferred, sorted);
}

#[test]
fn the_abox_is_merged_and_can_make_the_ontology_inconsistent() {
    let abox = ":alice a :Woman , :Man .";
    let r = run_with(ALCHIQ, Some(abox), None);
    assert_eq!(get(&r, "consistent"), &Value::Bool(false));
    assert_eq!(get(&r, "reason"), &Value::Null);
    // No classification is claimed for an inconsistent ontology.
    assert_eq!(get(&r, "classification"), &Value::Null);
    assert!(arr(get(&r, "unsatisfiable")).is_empty());
    assert!(arr(get(&r, "inferredSubClassOf")).is_empty());
    // A functional property with two distinct values: inconsistent through the ABox.
    let abox = ":a :hasMother :m1 , :m2 . :m1 owl:differentFrom :m2 .";
    let r = run_with(ALCHIQ, Some(abox), None);
    assert_eq!(get(&r, "consistent"), &Value::Bool(false));
    // And a harmless ABox keeps it consistent.
    let r = run_with(ALCHIQ, Some(":alice a :Mother ; :worksFor :acme ."), None);
    assert_eq!(get(&r, "consistent"), &Value::Bool(true));
}

#[test]
fn unsatisfiable_class_and_inferred_subclass() {
    let r = run(
        ":A a owl:Class . :B a owl:Class . :C a owl:Class . :D a owl:Class .
         :A rdfs:subClassOf :B , :C . :B owl:disjointWith :C .
         :D rdfs:subClassOf [ a owl:Class ; owl:intersectionOf ( :B :C ) ] ; owl:disjointWith :A .
         :E a owl:Class ; rdfs:subClassOf :F . :F a owl:Class ; rdfs:subClassOf :G . :G a owl:Class .",
    );
    assert_eq!(get(&r, "consistent"), &Value::Bool(true));
    assert_eq!(
        strs(get(&r, "unsatisfiable")),
        ["http://ex/A", "http://ex/D"]
    );
    assert_eq!(
        pairs(get(&r, "inferredSubClassOf")),
        [("http://ex/E", "http://ex/G")]
    );
}

#[test]
fn data_properties_are_out_of_fragment_with_the_offending_triple() {
    let r = run(&format!(
        "{ALCHIQ} :age a owl:DatatypeProperty ; rdfs:domain :Person ; rdfs:range xsd:integer ."
    ));
    assert_eq!(s(get(&r, "consistent")), "unknown");
    let reason = get(&r, "reason");
    assert_eq!(s(get(reason, "kind")), "OutOfFragment");
    assert!(!s(get(reason, "construct")).is_empty());
    let triples = strs(get(reason, "triples"));
    assert!(!triples.is_empty());
    assert!(
        triples.iter().all(|t| t.contains("<http://ex/age>")),
        "{triples:?}"
    );
    for t in &triples {
        assert!(t.ends_with(" ."), "{t}");
    }
    // The profile check still ran and the ontology is in OWL 2 DL; only reasoning abstains.
    assert_eq!(get(get(&r, "profile"), "dl"), &Value::Bool(true));
    assert_eq!(get(&r, "classification"), &Value::Null);
}

#[test]
fn extraction_refusal_is_a_result_not_an_error() {
    // A restriction with two fillers is recognisably OWL but ill-formed.
    let r = run(":A a owl:Class . :B a owl:Class . :r a owl:ObjectProperty .
         :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ;
             owl:someValuesFrom :B ; owl:allValuesFrom :B ] .");
    assert_eq!(keys(&r), SHAPE);
    assert_eq!(s(get(&r, "consistent")), "unknown");
    let reason = get(&r, "reason");
    assert_eq!(s(get(reason, "kind")), "Malformed");
    assert!(!s(get(reason, "detail")).is_empty());
    let triples = strs(get(reason, "triples"));
    assert!(!triples.is_empty());
    assert!(triples.iter().all(|t| t.starts_with("_:")), "{triples:?}");
    let profile = get(&r, "profile");
    assert_eq!(s(get(profile, "dl")), "unknown");
    assert_eq!(get(&r, "classification"), &Value::Null);
}

#[test]
fn profile_violation_blocks_reasoning() {
    // A transitive (non-simple) property under a cardinality restriction (§11.1).
    let r = run(
        ":A a owl:Class . :r a owl:ObjectProperty , owl:TransitiveProperty .
         :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ;
             owl:maxCardinality \"1\"^^xsd:nonNegativeInteger ] .",
    );
    assert_eq!(s(get(&r, "consistent")), "unknown");
    let reason = get(&r, "reason");
    assert_eq!(s(get(reason, "kind")), "NotInDl");
    let blocking = arr(get(reason, "violations"));
    assert_eq!(s(get(&blocking[0], "kind")), "NonSimpleProperty");
    let profile = get(&r, "profile");
    assert_eq!(get(profile, "dl"), &Value::Bool(false));
    let violations = arr(get(profile, "violations"));
    let v = violations
        .iter()
        .find(|v| s(get(v, "kind")) == "NonSimpleProperty")
        .expect("reported in the profile");
    assert!(s(get(v, "detail")).contains("http://ex/r"));
    assert!(!strs(get(v, "triples")).is_empty());
}

#[test]
fn imports_and_budget_reasons() {
    let r = run(":A a owl:Class . <http://ex/o> a owl:Ontology ; owl:imports <http://ex/other> .");
    assert_eq!(s(get(&r, "consistent")), "unknown");
    let reason = get(&r, "reason");
    assert_eq!(s(get(reason, "kind")), "ImportsNotLoaded");
    assert_eq!(strs(get(reason, "imports")), ["http://ex/other"]);
    assert_eq!(s(get(get(&r, "profile"), "dl")), "unknown");
    assert!(!arr(get(get(&r, "profile"), "unchecked")).is_empty());

    let tiny = Some(r#"{"budget":{"maxSteps":1}}"#);
    // The empty ABox is decided within one step; classifying is not.
    let r = run_with(ALCHIQ, None, tiny);
    assert_eq!(get(&r, "consistent"), &Value::Bool(true));
    assert_eq!(
        s(get(get(&r, "classification"), "unknown")),
        "ResourceBudget"
    );
    assert!(arr(get(&r, "inferredSubClassOf")).is_empty());
    // With individuals, consistency itself runs out of budget.
    let r = run_with(ALCHIQ, Some(":alice a :Mother ."), tiny);
    assert_eq!(s(get(&r, "consistent")), "unknown");
    assert_eq!(s(get(get(&r, "reason"), "kind")), "ResourceBudget");
    assert_eq!(get(&r, "classification"), &Value::Null);
}

#[test]
fn errors_are_parse_errors_and_bad_options_only() {
    let e = check_owl_dl_json("<a> <b> .", None, "ntriples", None).unwrap_err();
    assert!(e.starts_with("TBox parse error"), "{e}");
    let e = check_owl_dl_json(PRE, Some("this is not turtle"), "turtle", None).unwrap_err();
    assert!(e.starts_with("ABox parse error"), "{e}");
    assert!(check_owl_dl_json(PRE, None, "rdf/xml-ish", None).is_err());
    assert!(check_owl_dl_json(PRE, None, "turtle", Some(r#"{"budget":{"steps":1}}"#)).is_err());
    // An empty format means Turtle; an empty document is a consistent empty ontology.
    let out = check_owl_dl_json(PRE, None, "", None).unwrap();
    assert_eq!(get(&parse(&out).unwrap(), "consistent"), &Value::Bool(true));
}

#[test]
fn ntriples_input() {
    let sub = "<http://www.w3.org/2000/01/rdf-schema#subClassOf>";
    let ty = "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>";
    let class = "<http://www.w3.org/2002/07/owl#Class>";
    let doc = format!(
        "<http://ex/A> {ty} {class} .\n<http://ex/B> {ty} {class} .\n<http://ex/C> {ty} {class} .\n\
         <http://ex/A> {sub} <http://ex/B> .\n<http://ex/B> {sub} <http://ex/C> .\n"
    );
    let out = check_owl_dl_json(&doc, None, "ntriples", None).unwrap();
    let r = parse(&out).unwrap();
    assert_eq!(get(&r, "consistent"), &Value::Bool(true));
    assert_eq!(
        pairs(get(&r, "inferredSubClassOf")),
        [("http://ex/A", "http://ex/C")]
    );
}

#[test]
fn abox_blank_nodes_do_not_join_tbox_blank_nodes() {
    // `_:x` in the TBox and `_:x` in the ABox are different nodes: merged naively, `_:x`
    // would be both an A and a B, which are disjoint.
    let tbox = ":A a owl:Class . :B a owl:Class . :A owl:disjointWith :B . _:x a :A .";
    let r = run_with(tbox, Some("_:x a :B ."), None);
    assert_eq!(get(&r, "consistent"), &Value::Bool(true));
    // Within one document the label is one node.
    let r = run_with(tbox, Some("_:y a :B . _:y a :A ."), None);
    assert_eq!(get(&r, "consistent"), &Value::Bool(false));
}

#[test]
fn rendered_triples_escape_hostile_literals() {
    // An ill-typed literal with quotes, a backslash and a newline is reported with its
    // triple; the JSON stays well-formed and the N-Triples line is escaped.
    let r =
        run(":p a owl:AnnotationProperty . :A a owl:Class ; :p \"x\\\"y\\\\z\\nw\"^^xsd:integer .");
    let violations = arr(get(get(&r, "profile"), "violations"));
    let v = violations
        .iter()
        .find(|v| s(get(v, "kind")) == "IllTypedLiteral")
        .expect("ill-typed literal reported");
    let triples = strs(get(v, "triples"));
    assert_eq!(triples.len(), 1);
    assert!(triples[0].contains(r#""x\"y\\z\nw""#), "{}", triples[0]);
}
