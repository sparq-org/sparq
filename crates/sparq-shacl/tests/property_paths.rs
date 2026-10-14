//! End-to-end SHACL property-path coverage (sq-qap0).
//!
//! Each SHACL path form (predicate, inverse, sequence, alternative,
//! zeroOrMore, oneOrMore, zeroOrOne, and nested combinations) is exercised
//! through `validate()` over small shapes + data graphs with HAND-COMPUTED
//! conforming / violating focus nodes — positive AND negative cases. These
//! drive the path value-node selection in `path.rs` through the constraint
//! pipeline in `eval.rs` (the two lowest-covered files in the crate), and check
//! the reported `sh:value` / count where the spec fixes it.

use sparq_core::Graph;
use sparq_shacl::{validate, ValidationReport};

const PREFIXES: &str = r#"
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    @prefix ex: <http://example.org/> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

fn run(data: &str, shapes: &str) -> ValidationReport {
    let data = Graph::load_str(&format!("{PREFIXES}{data}"), "turtle").unwrap();
    let shapes = Graph::load_str(&format!("{PREFIXES}{shapes}"), "turtle").unwrap();
    validate(&data, &shapes)
}

/// The local names of the value nodes flagged in the report (the offending
/// `sh:value`s), sorted.
fn flagged_values(r: &ValidationReport) -> Vec<String> {
    let mut v: Vec<String> = r
        .results
        .iter()
        .filter_map(|x| x.value.as_ref().map(|t| t.to_string()))
        .collect();
    v.sort();
    v
}

fn ex(local: &str) -> String {
    format!("<http://example.org/{local}>")
}

// ---- predicate path ----

#[test]
fn predicate_path_class_constraint() {
    // sh:path ex:knows ; sh:class ex:Person. The value nodes are ex:knows
    // objects; non-Person ones are flagged.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:alice ;
          sh:property [ sh:path ex:knows ; sh:class ex:Person ] .
    "#;
    // Positive: both friends are Persons.
    let ok = run(
        r#"
        ex:alice ex:knows ex:bob , ex:carol .
        ex:bob a ex:Person . ex:carol a ex:Person .
    "#,
        shapes,
    );
    assert!(ok.conforms, "{}", ok.to_text());

    // Negative: ex:carol is not a Person — flagged as the offending value.
    let bad = run(
        r#"
        ex:alice ex:knows ex:bob , ex:carol .
        ex:bob a ex:Person .
    "#,
        shapes,
    );
    assert!(!bad.conforms);
    assert_eq!(flagged_values(&bad), vec![ex("carol")]);
}

// ---- inverse path ----

#[test]
fn inverse_path_min_count() {
    // [sh:inversePath ex:parent] from ex:alice = the things that have ex:alice
    // as their ex:parent (i.e. her children). minCount 1 requires >=1 child.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:alice , ex:bob ;
          sh:property [ sh:path [ sh:inversePath ex:parent ] ; sh:minCount 1 ] .
    "#;
    let r = run(
        r#"
        ex:child1 ex:parent ex:alice .
        ex:child2 ex:parent ex:alice .
        # ex:bob has no children -> minCount violation.
    "#,
        shapes,
    );
    assert!(!r.conforms);
    // Exactly one violation, on ex:bob (alice has two children, conforms).
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert_eq!(r.results[0].focus_node.to_string(), ex("bob"));
    assert!(r.results[0]
        .source_component
        .ends_with("MinCountConstraintComponent"));
}

// ---- sequence path ----

#[test]
fn sequence_path_node_kind() {
    // ( ex:knows ex:name ): the names of the people alice knows must be Literals.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:alice ;
          sh:property [ sh:path ( ex:knows ex:name ) ; sh:nodeKind sh:Literal ] .
    "#;
    // Positive.
    let ok = run(
        r#"
        ex:alice ex:knows ex:bob . ex:bob ex:name "Bob" .
    "#,
        shapes,
    );
    assert!(ok.conforms, "{}", ok.to_text());

    // Negative: bob's name is an IRI, not a literal — flagged.
    let bad = run(
        r#"
        ex:alice ex:knows ex:bob . ex:bob ex:name ex:bobIri .
    "#,
        shapes,
    );
    assert!(!bad.conforms);
    assert_eq!(flagged_values(&bad), vec![ex("bobIri")]);
    assert!(bad.results[0]
        .source_component
        .ends_with("NodeKindConstraintComponent"));
}

#[test]
fn sequence_path_dead_end_yields_no_values() {
    // A sequence whose first step has no match produces zero value nodes, so a
    // minCount on it fails (no values), and a maxCount passes (0 <= max).
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:alice ;
          sh:property [ sh:path ( ex:knows ex:name ) ; sh:minCount 1 ] .
    "#;
    let r = run("ex:alice ex:age 30 .", shapes); // no ex:knows at all
    assert!(!r.conforms);
    assert_eq!(r.results.len(), 1);
    assert!(r.results[0]
        .source_component
        .ends_with("MinCountConstraintComponent"));
}

// ---- alternative path ----

#[test]
fn alternative_path_dedups_and_counts() {
    // [sh:alternativePath ( ex:p1 ex:p2 )]: value nodes = union, deduped. With
    // ex:alice ex:p1 ex:x ; ex:p2 ex:x , ex:y -> values {ex:x, ex:y}.
    // maxCount 1 must therefore fail (2 > 1) — proving the dedup (3 triples but
    // 2 distinct values).
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:alice ;
          sh:property [ sh:path [ sh:alternativePath ( ex:p1 ex:p2 ) ] ; sh:maxCount 1 ] .
    "#;
    let r = run("ex:alice ex:p1 ex:x ; ex:p2 ex:x , ex:y .", shapes);
    assert!(!r.conforms);
    assert!(r.results[0]
        .source_component
        .ends_with("MaxCountConstraintComponent"));

    // Exactly one distinct value -> maxCount 1 conforms (the dedup makes the
    // doubled ex:x count once).
    let ok = run("ex:alice ex:p1 ex:x ; ex:p2 ex:x .", shapes);
    assert!(ok.conforms, "{}", ok.to_text());
}

// ---- zeroOrOne path ----

#[test]
fn zero_or_one_includes_focus_node() {
    // [sh:zeroOrOnePath ex:p]: value nodes = {focus} U {focus.p}. sh:class
    // ex:Thing: the focus itself must be a Thing too (it is in the value set).
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:a ;
          sh:property [ sh:path [ sh:zeroOrOnePath ex:p ] ; sh:class ex:Thing ] .
    "#;
    // Positive: both ex:a and its ex:p value are Things.
    let ok = run("ex:a a ex:Thing ; ex:p ex:b . ex:b a ex:Thing .", shapes);
    assert!(ok.conforms, "{}", ok.to_text());

    // Negative: ex:a itself is not a Thing -> the reflexive value (the focus)
    // is flagged.
    let bad = run("ex:a ex:p ex:b . ex:b a ex:Thing .", shapes);
    assert!(!bad.conforms);
    assert_eq!(flagged_values(&bad), vec![ex("a")]);
}

// ---- zeroOrMore path ----

#[test]
fn zero_or_more_transitive_closure() {
    // [sh:zeroOrMorePath ex:sub]: from ex:a over a -sub-> b -sub-> c, value
    // nodes = {a, b, c} (reflexive + transitive). sh:nodeKind sh:IRI: all are
    // IRIs -> conforms.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:a ;
          sh:property [ sh:path [ sh:zeroOrMorePath ex:sub ] ; sh:nodeKind sh:IRI ] .
    "#;
    let ok = run("ex:a ex:sub ex:b . ex:b ex:sub ex:c .", shapes);
    assert!(ok.conforms, "{}", ok.to_text());

    // hasValue ex:c: the closure from ex:a must contain ex:c -> conforms; from a
    // disconnected target it does not.
    let hv_shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:a , ex:lonely ;
          sh:property [ sh:path [ sh:zeroOrMorePath ex:sub ] ; sh:hasValue ex:c ] .
    "#;
    let r = run(
        "ex:a ex:sub ex:b . ex:b ex:sub ex:c . ex:lonely a ex:X .",
        hv_shapes,
    );
    assert!(!r.conforms);
    // Only ex:lonely fails hasValue (its closure is just {ex:lonely}); ex:a's
    // closure reaches ex:c.
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert_eq!(r.results[0].focus_node.to_string(), ex("lonely"));
    assert!(r.results[0]
        .source_component
        .ends_with("HasValueConstraintComponent"));
}

#[test]
fn zero_or_more_terminates_on_cycle() {
    // A cyclic chain a -sub-> b -sub-> a. zeroOrMore must terminate; value set
    // {a, b}. hasValue ex:b conforms (reachable); the validator must return.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:a ;
          sh:property [ sh:path [ sh:zeroOrMorePath ex:sub ] ; sh:hasValue ex:b ] .
    "#;
    let r = run("ex:a ex:sub ex:b . ex:b ex:sub ex:a .", shapes);
    assert!(
        r.conforms,
        "cyclic closure must terminate and conform: {}",
        r.to_text()
    );
}

// ---- oneOrMore path ----

#[test]
fn one_or_more_excludes_focus_unless_reachable() {
    // [sh:oneOrMorePath ex:sub]: NON-reflexive. From a -sub-> b -sub-> c value
    // nodes = {b, c} (NOT a). minCount 1 requires >=1 reachable node.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:a , ex:leaf ;
          sh:property [ sh:path [ sh:oneOrMorePath ex:sub ] ; sh:minCount 1 ] .
    "#;
    let r = run(
        "ex:a ex:sub ex:b . ex:b ex:sub ex:c . ex:leaf a ex:X .",
        shapes,
    );
    assert!(!r.conforms);
    // ex:leaf has no ex:sub successor -> 0 values -> minCount fails; ex:a has 2.
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert_eq!(r.results[0].focus_node.to_string(), ex("leaf"));
}

#[test]
fn one_or_more_vs_zero_or_one_distinct() {
    // Same data, the focus has NO outgoing ex:p. oneOrMore -> {} (minCount 1
    // fails); zeroOrOne -> {focus} (minCount 1 passes). Proves the reflexivity
    // distinction end-to-end.
    let data = "ex:a a ex:Thing .";
    let one_or_more = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:a ;
          sh:property [ sh:path [ sh:oneOrMorePath ex:p ] ; sh:minCount 1 ] .
    "#;
    let zero_or_one = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:a ;
          sh:property [ sh:path [ sh:zeroOrOnePath ex:p ] ; sh:minCount 1 ] .
    "#;
    assert!(
        !run(data, one_or_more).conforms,
        "oneOrMore should yield no values"
    );
    assert!(
        run(data, zero_or_one).conforms,
        "zeroOrOne includes the focus"
    );
}

// ---- nested combinations ----

#[test]
fn nested_sequence_of_inverse_and_star() {
    // ( [sh:inversePath ex:parent] [sh:zeroOrMorePath ex:knows] ):
    // from ex:root: inverse(parent) -> ex:kid (kid -parent-> root), then
    // zeroOrMore(knows) from ex:kid over kid -knows-> f1 -knows-> f2:
    // values = {kid, f1, f2}. sh:nodeKind sh:IRI -> all IRIs -> conforms.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:root ;
          sh:property [
            sh:path ( [ sh:inversePath ex:parent ] [ sh:zeroOrMorePath ex:knows ] ) ;
            sh:nodeKind sh:IRI ;
          ] .
    "#;
    let ok = run(
        r#"
        ex:kid ex:parent ex:root .
        ex:kid ex:knows ex:f1 . ex:f1 ex:knows ex:f2 .
    "#,
        shapes,
    );
    assert!(ok.conforms, "{}", ok.to_text());

    // Negative: a literal sneaks into the closure (f1 ex:knows "lit") and is
    // flagged by sh:nodeKind sh:IRI.
    let bad = run(
        r#"
        ex:kid ex:parent ex:root .
        ex:kid ex:knows ex:f1 . ex:f1 ex:knows "lit" .
    "#,
        shapes,
    );
    assert!(!bad.conforms);
    assert_eq!(flagged_values(&bad), vec![r#""lit""#.to_string()]);
}

#[test]
fn nested_alternative_of_sequences() {
    // [sh:alternativePath ( ( ex:a ex:b ) ex:c )]: value nodes are the union of
    // the a/b sequence and the direct ex:c. With x -a-> m -b-> n and x -c-> n,
    // values = {n} (deduped across both branches). minCount 1 conforms; a
    // sh:class ex:T flags n if it is not a T.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:x ;
          sh:property [
            sh:path [ sh:alternativePath ( ( ex:a ex:b ) ex:c ) ] ;
            sh:class ex:T ;
          ] .
    "#;
    // Positive: n is a T.
    let ok = run(
        "ex:x ex:a ex:m ; ex:c ex:n . ex:m ex:b ex:n . ex:n a ex:T .",
        shapes,
    );
    assert!(ok.conforms, "{}", ok.to_text());

    // Negative: n is not a T -> one violation (deduped value, reported once per
    // route: this nested alternative dedups so once).
    let bad = run("ex:x ex:a ex:m ; ex:c ex:n . ex:m ex:b ex:n .", shapes);
    assert!(!bad.conforms);
    assert_eq!(flagged_values(&bad), vec![ex("n")]);
}

// ---- the reported sh:resultPath carries the full path expression ----

#[test]
fn result_path_serialises_complex_path() {
    // A failing constraint on a complex path: the report's sh:resultPath must be
    // the SAME path expression (round-trips through the Turtle serialiser).
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:a ;
          sh:property [
            sh:path [ sh:zeroOrMorePath ex:sub ] ;
            sh:nodeKind sh:Literal ;
          ] .
    "#;
    // ex:a (an IRI) is in the value set and is not a Literal -> violation; the
    // result path must be the zeroOrMore expression.
    let r = run("ex:a ex:sub ex:b .", shapes);
    assert!(!r.conforms);
    let path = r.results[0].path.as_ref().expect("result has a path");
    assert_eq!(
        path.to_turtle(),
        "[ <http://www.w3.org/ns/shacl#zeroOrMorePath> <http://example.org/sub> ]"
    );
}
