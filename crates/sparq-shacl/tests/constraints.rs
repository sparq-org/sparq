//! [OPUS-4.8] Constraint-component coverage (sq-qap0) for the branches the
//! audit flagged in `eval.rs` (~58% covered). The crate's own (non-W3C) tests
//! exercised only datatype / nodeKind / minInclusive / class; the W3C suite
//! self-skips without fixtures, so these give the constraint pipeline durable,
//! fixture-free coverage with hand-computed conforming / violating outcomes.
//!
//! Each test pins the source component IRI and (where the spec fixes it) the
//! reported `sh:value` and the result count — the per-pair / per-occurrence
//! reporting policy `eval.rs` documents.

use oxrdf::Term;
use sparq_core::Graph;
use sparq_shacl::ValidationReport;

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
    sparq_shacl::validate(&data, &shapes)
}

/// Count of results whose source component IRI ends with `local`.
fn count_component(r: &ValidationReport, local: &str) -> usize {
    r.results
        .iter()
        .filter(|x| x.source_component.ends_with(local))
        .count()
}

fn flagged_values(r: &ValidationReport) -> Vec<String> {
    let mut v: Vec<String> = r
        .results
        .iter()
        .filter_map(|x| x.value.as_ref().map(|t| t.to_string()))
        .collect();
    v.sort();
    v
}

// ---- sh:equals (symmetric: missing-in-either direction) ----

#[test]
fn equals_reports_both_directions() {
    // sh:path ex:a ; sh:equals ex:b. Value sets must be EQUAL. Here ex:a = {1,2}
    // and ex:b = {2,3}: 1 is in a-not-b, 3 is in b-not-a -> two results.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:a ; sh:equals ex:b ] .
    "#;
    let r = run("ex:n ex:a 1 , 2 ; ex:b 2 , 3 .", shapes);
    assert!(!r.conforms);
    assert_eq!(
        count_component(&r, "EqualsConstraintComponent"),
        2,
        "{}",
        r.to_text()
    );

    // Equal sets conform.
    let ok = run("ex:n ex:a 1 , 2 ; ex:b 1 , 2 .", shapes);
    assert!(ok.conforms, "{}", ok.to_text());
}

// ---- sh:disjoint ----

#[test]
fn disjoint_flags_shared_values() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:a ; sh:disjoint ex:b ] .
    "#;
    // ex:shared appears under both -> one disjoint violation on that value.
    let r = run(
        "ex:n ex:a ex:shared , ex:x ; ex:b ex:shared , ex:y .",
        shapes,
    );
    assert!(!r.conforms);
    assert_eq!(count_component(&r, "DisjointConstraintComponent"), 1);
    assert_eq!(
        flagged_values(&r),
        vec!["<http://example.org/shared>".to_string()]
    );

    // No overlap conforms.
    let ok = run("ex:n ex:a ex:x ; ex:b ex:y .", shapes);
    assert!(ok.conforms);
}

// ---- sh:lessThan / sh:lessThanOrEquals (one result per failing (value,other) pair) ----

#[test]
fn less_than_reports_per_pair() {
    // sh:path ex:v ; sh:lessThan ex:limit. ex:v = {5}; ex:limit = {3, 4}. 5 is
    // not < 3 and not < 4 -> TWO results, both sh:value 5.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ; sh:lessThan ex:limit ] .
    "#;
    let r = run("ex:n ex:v 5 ; ex:limit 3 , 4 .", shapes);
    assert!(!r.conforms);
    assert_eq!(
        count_component(&r, "LessThanConstraintComponent"),
        2,
        "{}",
        r.to_text()
    );
    assert_eq!(
        flagged_values(&r),
        vec![
            "\"5\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string(),
            "\"5\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string(),
        ]
    );

    // 1 < 3 and 1 < 4 -> conforms.
    let ok = run("ex:n ex:v 1 ; ex:limit 3 , 4 .", shapes);
    assert!(ok.conforms);
}

#[test]
fn less_than_or_equals_allows_equal() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ; sh:lessThanOrEquals ex:limit ] .
    "#;
    // 3 <= 3 conforms under OrEquals.
    let ok = run("ex:n ex:v 3 ; ex:limit 3 .", shapes);
    assert!(ok.conforms, "{}", ok.to_text());
    // 4 <= 3 fails.
    let bad = run("ex:n ex:v 4 ; ex:limit 3 .", shapes);
    assert!(!bad.conforms);
    assert_eq!(
        count_component(&bad, "LessThanOrEqualsConstraintComponent"),
        1
    );
}

// ---- range components across string / boolean / dateTime orderings ----

#[test]
fn range_string_ordering() {
    // sh:minInclusive "m" on strings: "a" < "m" fails, "z" >= "m" passes.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:s ; sh:minInclusive "m" ] .
    "#;
    let r = run(r#"ex:n ex:s "a" , "z" ."#, shapes);
    assert!(!r.conforms);
    assert_eq!(count_component(&r, "MinInclusiveConstraintComponent"), 1);
    assert_eq!(flagged_values(&r), vec!["\"a\"".to_string()]);
}

#[test]
fn range_datetime_ordering_and_incomparability() {
    // sh:maxInclusive a timezoned dateTime. A tz-less value compares via
    // XSD's ±14h rule: determinate when its 28h instant window falls wholly
    // on one side of the bound, INCOMPARABLE inside the window (-> violation).
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:d ;
            sh:maxInclusive "2024-06-01T00:00:00Z"^^xsd:dateTime ] .
    "#;
    // Earlier timezoned value conforms.
    let ok = run(
        r#"ex:n ex:d "2024-01-01T00:00:00Z"^^xsd:dateTime ."#,
        shapes,
    );
    assert!(ok.conforms, "{}", ok.to_text());
    // A tz-less value months below the bound is determinately less -> conforms.
    let ok = run(r#"ex:n ex:d "2024-01-01T00:00:00"^^xsd:dateTime ."#, shapes);
    assert!(
        ok.conforms,
        "tz-less value below the ±14h window is determinately < the bound: {}",
        ok.to_text()
    );
    // A tz-less value inside the bound's ±14h window is incomparable ->
    // constraint not satisfied -> violation.
    let bad = run(r#"ex:n ex:d "2024-06-01T00:00:00"^^xsd:dateTime ."#, shapes);
    assert!(
        !bad.conforms,
        "tz-less vs tz'd dateTime inside ±14h must be incomparable"
    );
    assert_eq!(count_component(&bad, "MaxInclusiveConstraintComponent"), 1);
}

#[test]
fn range_max_exclusive_boundary() {
    // sh:maxExclusive 10: 10 fails (not < 10), 9 passes.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ; sh:maxExclusive 10 ] .
    "#;
    let r = run("ex:n ex:v 10 , 9 .", shapes);
    assert!(!r.conforms);
    assert_eq!(count_component(&r, "MaxExclusiveConstraintComponent"), 1);
    assert_eq!(
        flagged_values(&r),
        vec!["\"10\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string()]
    );
}

// ---- sh:minLength / sh:maxLength (char count, IRI vs literal vs blank) ----

#[test]
fn length_counts_characters_and_rejects_blank() {
    // sh:minLength 3 over mixed value kinds. "ab" (2) fails, "abc" (3) passes,
    // an IRI uses its full string (long, passes), a blank node has no string
    // repr (fails).
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ; sh:minLength 3 ] .
    "#;
    let r = run(r#"ex:n ex:v "ab" , "abc" , ex:longIri , [ ] ."#, shapes);
    assert!(!r.conforms);
    // "ab" and the blank node fail; "abc" and the IRI pass.
    assert_eq!(
        count_component(&r, "MinLengthConstraintComponent"),
        2,
        "{}",
        r.to_text()
    );
}

#[test]
fn max_length_unicode_chars_not_bytes() {
    // "é" + "é" via combining? Use a 2-char multibyte string vs maxLength 2.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ; sh:maxLength 2 ] .
    "#;
    // "αβ" is two chars (4 bytes) -> within maxLength 2.
    let ok = run("ex:n ex:v \"\u{3b1}\u{3b2}\" .", shapes);
    assert!(ok.conforms, "char-count, not byte-count: {}", ok.to_text());
    // Three chars exceeds.
    let bad = run("ex:n ex:v \"\u{3b1}\u{3b2}\u{3b3}\" .", shapes);
    assert!(!bad.conforms);
}

// ---- sh:pattern (+ sh:flags) ----

#[test]
fn pattern_with_flags() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:code ; sh:pattern "^ab" ; sh:flags "i" ] .
    "#;
    // "ABxyz" matches case-insensitively; "zab" does not (anchored at start).
    let r = run(r#"ex:n ex:code "ABxyz" , "zab" ."#, shapes);
    assert!(!r.conforms);
    assert_eq!(count_component(&r, "PatternConstraintComponent"), 1);
    assert_eq!(flagged_values(&r), vec!["\"zab\"".to_string()]);
}

/// [FABLE-5] (sq-8ro) XPath F&O `q` flag: literal-pattern mode — every pattern
/// character (here `.` and `+`) matches literally, so `"a.b+"` conforms while the
/// would-be regex match `"axbb"` violates.
#[test]
fn pattern_q_flag_is_literal_mode() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:code ; sh:pattern "a.b+" ; sh:flags "q" ] .
    "#;
    let r = run(r#"ex:n ex:code "xa.b+y" , "axbb" ."#, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(count_component(&r, "PatternConstraintComponent"), 1);
    assert_eq!(flagged_values(&r), vec!["\"axbb\"".to_string()]);
}

/// [FABLE-5] (sq-8ro) Per XPath F&O only `i` keeps its effect alongside `q`:
/// `"qi"` matches the literal pattern case-insensitively.
#[test]
fn pattern_q_flag_combines_with_i() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:code ; sh:pattern "A.B" ; sh:flags "qi" ] .
    "#;
    let r = run(r#"ex:n ex:code "a.b" , "AxB" ."#, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(flagged_values(&r), vec!["\"AxB\"".to_string()]);
}

/// [FABLE-5] (sq-8ro) XPath/XSD `\i` (XML NameStartChar) and `\c` (XML NameChar)
/// multi-character escapes, which the Rust `regex` crate does not know: an
/// XML-name-shaped pattern `^\i\c*$` accepts `abc` / `_a:b-c.1` / `édition`
/// (NameStartChar covers #xC0–#xD6) and rejects a digit-initial or
/// space-containing value.
#[test]
fn pattern_xpath_name_char_classes() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:code ; sh:pattern "^\\i\\c*$" ] .
    "#;
    let ok = run(
        "ex:n ex:code \"abc\" , \"_a:b-c.1\" , \"\u{e9}dition\" .",
        shapes,
    );
    assert!(ok.conforms, "XML-name values conform: {}", ok.to_text());
    assert!(ok.diagnostics.is_empty(), "no skip diagnostic: {:?}", ok.diagnostics);
    let bad = run(r#"ex:n ex:code "1abc" , "ab cd" ."#, shapes);
    assert!(!bad.conforms, "{}", bad.to_text());
    assert_eq!(count_component(&bad, "PatternConstraintComponent"), 2);
}

/// [FABLE-5] (sq-8ro) The complements `\I` / `\C`, and `\c` INSIDE a character
/// class (where it must expand to a nested class, not a bracketed group).
#[test]
fn pattern_xpath_name_class_complements_and_in_class() {
    // ^\C$ = exactly one non-NameChar: a space conforms, a letter violates.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:code ; sh:pattern "^\\C$" ] .
    "#;
    let r = run(r#"ex:n ex:code " " , "a" ."#, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(flagged_values(&r), vec!["\"a\"".to_string()]);

    // \c inside a class, unioned with "!": "a!" conforms, "a " violates.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:code ; sh:pattern "^[\\c!]+$" ] .
    "#;
    let r = run(r#"ex:n ex:code "a!" , "a " ."#, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(flagged_values(&r), vec!["\"a \"".to_string()]);

    // An ESCAPED backslash before `i` stays a literal-backslash match — the
    // translator must not misread `\\i` as the name-class escape.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:code ; sh:pattern "^\\\\i$" ] .
    "#;
    let r = run(r#"ex:n ex:code "\\i" , "x" ."#, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(flagged_values(&r), vec!["\"x\"".to_string()]);
}

// ---- sh:languageIn / sh:uniqueLang ----

#[test]
fn language_in_basic_range_match() {
    // sh:languageIn ("en" "fr"): "en-GB" matches the "en" range; "de" does not;
    // a non-language-tagged literal fails.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:label ; sh:languageIn ( "en" "fr" ) ] .
    "#;
    let r = run(
        r#"ex:n ex:label "hello"@en-GB , "bonjour"@fr , "hallo"@de , "plain" ."#,
        shapes,
    );
    assert!(!r.conforms);
    // "de" and the plain literal fail; "en-GB" and "fr" pass.
    assert_eq!(
        count_component(&r, "LanguageInConstraintComponent"),
        2,
        "{}",
        r.to_text()
    );
}

#[test]
fn unique_lang_flags_duplicate_language() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:label ; sh:uniqueLang true ] .
    "#;
    // Two @en values -> one uniqueLang violation (no sh:value).
    let r = run(r#"ex:n ex:label "a"@en , "b"@en , "c"@fr ."#, shapes);
    assert!(!r.conforms);
    assert_eq!(count_component(&r, "UniqueLangConstraintComponent"), 1);
    assert!(
        r.results[0].value.is_none(),
        "uniqueLang reports no sh:value"
    );

    // Distinct languages conform.
    let ok = run(r#"ex:n ex:label "a"@en , "b"@fr ."#, shapes);
    assert!(ok.conforms);
}

// ---- sh:in / sh:hasValue ----

#[test]
fn in_enumeration_value_semantics() {
    // sh:in ( 1 2 ) uses value equality: an int 1 is in; 3 is not.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ; sh:in ( 1 2 ) ] .
    "#;
    let r = run("ex:n ex:v 1 , 3 .", shapes);
    assert!(!r.conforms);
    assert_eq!(count_component(&r, "InConstraintComponent"), 1);
    assert_eq!(
        flagged_values(&r),
        vec!["\"3\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string()]
    );
}

#[test]
fn has_value_requires_presence() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n , ex:m ;
          sh:property [ sh:path ex:status ; sh:hasValue ex:active ] .
    "#;
    let r = run(
        "ex:n ex:status ex:active , ex:x . ex:m ex:status ex:x .",
        shapes,
    );
    assert!(!r.conforms);
    // ex:m lacks ex:active -> one violation (no sh:value); ex:n conforms.
    assert_eq!(count_component(&r, "HasValueConstraintComponent"), 1);
    assert_eq!(
        r.results[0].focus_node.to_string(),
        "<http://example.org/m>"
    );
    assert!(r.results[0].value.is_none());
}

// ---- sh:closed (+ sh:ignoredProperties) ----

#[test]
fn closed_flags_extra_predicates_and_honours_ignored() {
    // A closed node shape allowing only ex:name (declared via sh:property) and
    // ignoring rdf:type. An extra ex:age predicate is flagged; rdf:type is not.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:closed true ;
          sh:ignoredProperties ( rdf:type ) ;
          sh:property [ sh:path ex:name ] .
    "#;
    let r = run(
        r#"ex:n a ex:Thing ; ex:name "N" ; ex:age 5 ; ex:extra 1 ."#,
        shapes,
    );
    assert!(!r.conforms);
    // ex:age and ex:extra are disallowed (rdf:type ignored, ex:name allowed).
    assert_eq!(
        count_component(&r, "ClosedConstraintComponent"),
        2,
        "{}",
        r.to_text()
    );
    // The closed result carries the offending predicate as the result path.
    let paths: Vec<String> = r
        .results
        .iter()
        .filter(|x| x.source_component.ends_with("ClosedConstraintComponent"))
        .filter_map(|x| x.path.as_ref().map(|p| p.to_turtle()))
        .collect();
    assert!(paths.iter().any(|p| p.contains("age")), "{paths:?}");
    assert!(paths.iter().any(|p| p.contains("extra")), "{paths:?}");

    // Only allowed/ignored predicates conform.
    let ok = run(r#"ex:n a ex:Thing ; ex:name "N" ."#, shapes);
    assert!(ok.conforms, "{}", ok.to_text());
}

// ---- logical: sh:not / sh:and / sh:or / sh:xone ----

#[test]
fn logical_not() {
    // sh:not [ sh:datatype xsd:string ]: a string value VIOLATES (it conforms to
    // the negated shape); an integer conforms.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ; sh:not [ sh:datatype xsd:string ] ] .
    "#;
    let bad = run(r#"ex:n ex:v "hi" ."#, shapes);
    assert!(!bad.conforms);
    assert_eq!(count_component(&bad, "NotConstraintComponent"), 1);
    let ok = run("ex:n ex:v 5 .", shapes);
    assert!(ok.conforms);
}

#[test]
fn logical_and_or_xone() {
    // sh:and ( [minInclusive 0] [maxInclusive 10] ): 5 conforms, 20 fails (one
    // And violation on the value).
    let and_shape = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ;
            sh:and ( [ sh:minInclusive 0 ] [ sh:maxInclusive 10 ] ) ] .
    "#;
    assert!(run("ex:n ex:v 5 .", and_shape).conforms);
    let and_bad = run("ex:n ex:v 20 .", and_shape);
    assert!(!and_bad.conforms);
    assert_eq!(count_component(&and_bad, "AndConstraintComponent"), 1);

    // sh:or ( [datatype xsd:string] [datatype xsd:integer] ): a boolean matches
    // neither -> one Or violation.
    let or_shape = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ;
            sh:or ( [ sh:datatype xsd:string ] [ sh:datatype xsd:integer ] ) ] .
    "#;
    assert!(run("ex:n ex:v 5 .", or_shape).conforms);
    assert!(run(r#"ex:n ex:v "x" ."#, or_shape).conforms);
    let or_bad = run("ex:n ex:v true .", or_shape);
    assert!(!or_bad.conforms);
    assert_eq!(count_component(&or_bad, "OrConstraintComponent"), 1);

    // sh:xone ( [datatype xsd:string] [minLength 1] ): a string matches BOTH ->
    // xone (exactly one) fails; an integer matches NEITHER -> also fails; an IRI
    // of length>=1 matches only minLength -> conforms.
    let xone_shape = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ;
            sh:xone ( [ sh:datatype xsd:string ] [ sh:nodeKind sh:IRI ] ) ] .
    "#;
    // a string matches only the datatype branch -> exactly one -> conforms.
    assert!(run(r#"ex:n ex:v "s" ."#, xone_shape).conforms);
    // an IRI matches only the nodeKind branch -> exactly one -> conforms.
    assert!(run("ex:n ex:v ex:thing .", xone_shape).conforms);
    // an integer matches neither -> xone fails.
    let xone_bad = run("ex:n ex:v 7 .", xone_shape);
    assert!(!xone_bad.conforms);
    assert_eq!(count_component(&xone_bad, "XoneConstraintComponent"), 1);
}

// ---- sh:qualifiedValueShape (+ min/max + disjoint) ----

#[test]
fn qualified_value_shape_min_max() {
    // At least 2 of the ex:item values must be ex:Special.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:item ;
            sh:qualifiedValueShape [ sh:class ex:Special ] ;
            sh:qualifiedMinCount 2 ] .
    "#;
    // Only one Special -> minCount violation.
    let bad = run("ex:n ex:item ex:a , ex:b . ex:a a ex:Special .", shapes);
    assert!(!bad.conforms);
    assert_eq!(
        count_component(&bad, "QualifiedMinCountConstraintComponent"),
        1
    );

    // Two Special -> conforms.
    let ok = run(
        "ex:n ex:item ex:a , ex:b . ex:a a ex:Special . ex:b a ex:Special .",
        shapes,
    );
    assert!(ok.conforms, "{}", ok.to_text());
}

#[test]
fn qualified_max_count() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:item ;
            sh:qualifiedValueShape [ sh:class ex:Special ] ;
            sh:qualifiedMaxCount 1 ] .
    "#;
    // Two Special exceeds max 1 -> violation.
    let bad = run(
        "ex:n ex:item ex:a , ex:b . ex:a a ex:Special . ex:b a ex:Special .",
        shapes,
    );
    assert!(!bad.conforms);
    assert_eq!(
        count_component(&bad, "QualifiedMaxCountConstraintComponent"),
        1
    );
}

// ---- sh:node / sh:property nesting reporting ----

#[test]
fn node_constraint_on_path_values() {
    // sh:path ex:addr ; sh:node ex:AddrShape requiring ex:city minCount 1.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:addr ; sh:node ex:AddrShape ] .
        ex:AddrShape a sh:NodeShape ;
          sh:property [ sh:path ex:city ; sh:minCount 1 ] .
    "#;
    // The address has no city -> the NodeConstraintComponent flags the addr value.
    let bad = run("ex:n ex:addr ex:addr1 .", shapes);
    assert!(!bad.conforms);
    assert_eq!(count_component(&bad, "NodeConstraintComponent"), 1);
    assert_eq!(
        flagged_values(&bad),
        vec!["<http://example.org/addr1>".to_string()]
    );

    let ok = run(
        r#"ex:n ex:addr ex:addr1 . ex:addr1 ex:city "NYC" ."#,
        shapes,
    );
    assert!(ok.conforms, "{}", ok.to_text());
}

// ---- targets: subjectsOf / objectsOf ----

#[test]
fn target_subjects_and_objects_of() {
    // sh:targetSubjectsOf ex:owns: every subject of an ex:owns triple is a focus
    // node, required to have a name.
    let subj_shape = r#"
        ex:S a sh:NodeShape ; sh:targetSubjectsOf ex:owns ;
          sh:property [ sh:path ex:name ; sh:minCount 1 ] .
    "#;
    let r = run(
        r#"ex:alice ex:owns ex:car . ex:bob ex:owns ex:bike ; ex:name "Bob" ."#,
        subj_shape,
    );
    assert!(!r.conforms);
    // ex:alice (subject, no name) fails; ex:bob conforms.
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert_eq!(
        r.results[0].focus_node.to_string(),
        "<http://example.org/alice>"
    );

    // sh:targetObjectsOf ex:owns: every OBJECT of ex:owns must be an ex:Vehicle.
    let obj_shape = r#"
        ex:S a sh:NodeShape ; sh:targetObjectsOf ex:owns ;
          sh:nodeKind sh:IRI ;
          sh:class ex:Vehicle .
    "#;
    let r = run(
        "ex:alice ex:owns ex:car . ex:car a ex:Vehicle . ex:bob ex:owns ex:notV .",
        obj_shape,
    );
    assert!(!r.conforms);
    // ex:notV is an object that is not a Vehicle.
    assert_eq!(count_component(&r, "ClassConstraintComponent"), 1);
    assert_eq!(
        r.results[0].focus_node.to_string(),
        "<http://example.org/notV>"
    );
}

// ---- sh:deactivated ----

#[test]
fn deactivated_shape_is_skipped() {
    // A shape that would fail, but sh:deactivated true -> no results.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:deactivated true ;
          sh:property [ sh:path ex:name ; sh:minCount 1 ] .
    "#;
    let r = run("ex:n a ex:Thing .", shapes);
    assert!(
        r.conforms,
        "deactivated shape must not produce results: {}",
        r.to_text()
    );
}

// ---- [OPUS-4.8] (sq-vg3y) SHACL-1.2 core/node forms ----

// ---- sh:datatype disjunctive (list) form ----

#[test]
fn datatype_list_form_accepts_any_listed() {
    // sh:datatype ( xsd:string rdf:langString ): a plain string and a lang-tagged
    // literal both conform; an integer (neither) violates. Mirrors W3C datatype-003.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode "plain" ;
          sh:targetNode "tagged"@en ;
          sh:targetNode 42 ;
          sh:datatype ( xsd:string rdf:langString ) .
    "#;
    let r = run("", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(count_component(&r, "DatatypeConstraintComponent"), 1);
    // The integer is the sole violator.
    assert!(flagged_values(&r).iter().any(|v| v.contains("42")));
}

#[test]
fn datatype_single_iri_still_works() {
    // Regression: the single-IRI form is the singleton-set case.
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode "x" , 1 ;
          sh:datatype xsd:string .
    "#;
    let r = run("", shapes);
    assert_eq!(count_component(&r, "DatatypeConstraintComponent"), 1);
}

// ---- sh:nodeKind disjunctive (list) form ----

#[test]
fn nodekind_list_form_accepts_iri_or_blanknode() {
    // sh:nodeKind ( sh:BlankNode sh:IRI ): an IRI and a blank node conform; a
    // literal violates. Mirrors W3C nodeKind-002.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:iri ;
          sh:targetNode "true"^^xsd:boolean ;
          sh:nodeKind ( sh:BlankNode sh:IRI ) .
    "#;
    let r = run("ex:iri ex:p 0 .", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(count_component(&r, "NodeKindConstraintComponent"), 1);
}

// ---- sh:closed sh:ByTypes ----

#[test]
fn closed_by_types_uses_properties_of_value_node_types() {
    // closed-003 in miniature: ex:RootClass is closed-by-types and declares
    // ex:rootClassProperty1; ex:SubClass (subclass) declares ex:subClassProperty1.
    // An instance of ROOT (not Sub) that uses ex:subClassProperty1 is closed out;
    // an instance of Sub that uses it is allowed (its type pulls the property in).
    let shapes = r#"
        ex:RootClass a rdfs:Class, sh:NodeShape ;
          sh:property [ sh:path ex:rootClassProperty1 ] ;
          sh:closed sh:ByTypes .
        ex:SubClass a rdfs:Class, sh:NodeShape ;
          rdfs:subClassOf ex:RootClass ;
          sh:property [ sh:path ex:subClassProperty1 ] ;
          sh:closed sh:ByTypes .
    "#;
    let data = r#"
        ex:RootInstance a ex:RootClass ; ex:subClassProperty1 1 .
        ex:SubInstance a ex:SubClass ; ex:rootClassProperty1 1 ; ex:subClassProperty1 3 .
    "#;
    let r = run(data, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    // Exactly one closed violation: ex:RootInstance's ex:subClassProperty1.
    assert_eq!(count_component(&r, "ClosedConstraintComponent"), 1);
    let closed: Vec<_> = r
        .results
        .iter()
        .filter(|x| x.source_component.ends_with("ClosedConstraintComponent"))
        .collect();
    assert!(closed[0].focus_node.to_string().contains("RootInstance"));
}

// ---- sh:memberShape ----

#[test]
fn member_shape_checks_each_list_member() {
    // sh:memberShape [ sh:nodeKind sh:IRI ]: a list of IRIs conforms; a list with
    // a literal member, AND a value that is not a SHACL list at all, both violate.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:goodList, ex:badList, ex:notAList ;
          sh:memberShape [ sh:nodeKind sh:IRI ] .
    "#;
    let data = r#"
        ex:goodList rdf:first ex:a ; rdf:rest ( ex:b ) .
        ex:badList  rdf:first ex:a ; rdf:rest ( "lit" ) .
        ex:notAList ex:p 0 .
    "#;
    let r = run(data, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    // One top-level result per violating value node (badList + notAList).
    assert_eq!(count_component(&r, "MemberShapeConstraintComponent"), 2);
}

// [OPUS-4.8] (sq-f8gu) sh:memberShape emits one sh:detail sub-result per
// NON-CONFORMING member: the actual validation results of validating that member
// against the member shape. A non-list value node carries NO details (there are
// no members to validate against the shape).
#[test]
fn member_shape_emits_detail_per_failing_member() {
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:list, ex:notAList ;
          sh:memberShape [ sh:nodeKind sh:IRI ] .
    "#;
    // ex:list = ( ex:ok "bad1" "bad2" ): one IRI member (ok) and two literal
    // members (each violates sh:nodeKind sh:IRI).
    let data = r#"
        ex:list rdf:first ex:ok ; rdf:rest ( "bad1" "bad2" ) .
        ex:notAList ex:p 0 .
    "#;
    let r = run(data, shapes);
    assert!(!r.conforms, "{}", r.to_text());

    let top: Vec<_> = r
        .results
        .iter()
        .filter(|x| {
            x.source_component
                .ends_with("MemberShapeConstraintComponent")
        })
        .collect();
    assert_eq!(top.len(), 2, "{}", r.to_text());

    // The list result carries two details (one per failing member); each detail
    // is a NodeKind violation whose sh:value is the offending literal.
    let list_result = top
        .iter()
        .find(|x| {
            x.value
                .as_ref()
                .map(std::string::ToString::to_string)
                .as_deref()
                == Some("<http://example.org/list>")
        })
        .expect("a result on ex:list");
    assert_eq!(list_result.details.len(), 2, "{}", r.to_text());
    assert!(list_result
        .details
        .iter()
        .all(|d| d.source_component.ends_with("NodeKindConstraintComponent")));
    let mut detail_vals: Vec<String> = list_result
        .details
        .iter()
        .filter_map(|d| d.value.as_ref().map(std::string::ToString::to_string))
        .collect();
    detail_vals.sort();
    assert_eq!(
        detail_vals,
        vec![r#""bad1""#.to_string(), r#""bad2""#.to_string()]
    );

    // The non-list value node carries no details.
    let notalist = top
        .iter()
        .find(|x| {
            x.value
                .as_ref()
                .map(std::string::ToString::to_string)
                .as_deref()
                == Some("<http://example.org/notAList>")
        })
        .expect("a result on ex:notAList");
    assert!(notalist.details.is_empty(), "{}", r.to_text());

    // sh:detail must surface in the Turtle report and round-trip as valid Turtle.
    let ttl = r.to_turtle();
    assert!(ttl.contains("sh:detail"), "no sh:detail in report: {ttl}");
    oxttl::TurtleParser::new()
        .for_slice(ttl.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| panic!("report Turtle does not parse: {e}\n{ttl}"));
}

// ---- sh:uniqueMembers ----

#[test]
fn unique_members_flags_duplicate_and_nonlist() {
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:uniq, ex:dup, ex:notAList ;
          sh:uniqueMembers true .
    "#;
    let data = r#"
        ex:uniq rdf:first 1 ; rdf:rest ( 2 3 ) .
        ex:dup  rdf:first 1 ; rdf:rest ( 2 1 ) .
        ex:notAList ex:p 0 .
    "#;
    let r = run(data, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    // ex:dup (duplicate 1) + ex:notAList (not a list) -> two results; ex:uniq ok.
    assert_eq!(count_component(&r, "UniqueMembersConstraintComponent"), 2);
}

// [OPUS-4.8] (sq-f8gu) sh:uniqueMembers emits one sh:detail sub-result per
// DUPLICATED member (each duplicated value reported once via sh:value), and a
// non-list value node carries no details.
#[test]
fn unique_members_emits_detail_per_duplicate() {
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:dup, ex:notAList ;
          sh:uniqueMembers true .
    "#;
    // ex:dup = ( 1 2 1 3 2 ): members 1 and 2 each appear twice -> two duplicates.
    let data = r#"
        ex:dup rdf:first 1 ; rdf:rest ( 2 1 3 2 ) .
        ex:notAList ex:p 0 .
    "#;
    let r = run(data, shapes);
    assert!(!r.conforms, "{}", r.to_text());

    let dup_result = r
        .results
        .iter()
        .find(|x| {
            x.source_component
                .ends_with("UniqueMembersConstraintComponent")
                && x.value
                    .as_ref()
                    .map(std::string::ToString::to_string)
                    .as_deref()
                    == Some("<http://example.org/dup>")
        })
        .expect("a result on ex:dup");
    // One detail per duplicated value (1 and 2), each carrying that value.
    assert_eq!(dup_result.details.len(), 2, "{}", r.to_text());
    let mut detail_vals: Vec<String> = dup_result
        .details
        .iter()
        .filter_map(|d| d.value.as_ref().map(std::string::ToString::to_string))
        .collect();
    detail_vals.sort();
    assert_eq!(
        detail_vals,
        vec![
            r#""1"^^<http://www.w3.org/2001/XMLSchema#integer>"#.to_string(),
            r#""2"^^<http://www.w3.org/2001/XMLSchema#integer>"#.to_string(),
        ]
    );

    let notalist = r
        .results
        .iter()
        .find(|x| {
            x.source_component
                .ends_with("UniqueMembersConstraintComponent")
                && x.value
                    .as_ref()
                    .map(std::string::ToString::to_string)
                    .as_deref()
                    == Some("<http://example.org/notAList>")
        })
        .expect("a result on ex:notAList");
    assert!(notalist.details.is_empty(), "{}", r.to_text());
}

// ---- sh:maxListLength / sh:minListLength ----

#[test]
fn max_list_length_flags_too_long_and_nonlist() {
    // Value nodes: ex:ok (2 members, ok), rdf:nil (0, ok), ex:long (3, violates),
    // ex:notAList (not a list, violates). Mirrors W3C maxListLength-001.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:ok, rdf:nil, ex:long, ex:notAList ;
          sh:maxListLength 2 .
    "#;
    let data = r#"
        ex:ok   rdf:first 1 ; rdf:rest ( 2 ) .
        ex:long rdf:first 1 ; rdf:rest ( 2 3 ) .
        ex:notAList ex:p 0 .
    "#;
    let r = run(data, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(count_component(&r, "MaxListLengthConstraintComponent"), 2);
}

#[test]
fn min_list_length_flags_too_short_and_nil() {
    // rdf:nil is a valid SHACL list of length 0 < 1 -> violates; ex:notAList is
    // not a list -> violates; ex:ok (length 2) passes. Mirrors W3C minListLength-001.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetNode ex:ok, rdf:nil, ex:notAList ;
          sh:minListLength 1 .
    "#;
    let data = r#"
        ex:ok rdf:first 1 ; rdf:rest ( 2 ) .
        ex:notAList ex:p 0 .
    "#;
    let r = run(data, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(count_component(&r, "MinListLengthConstraintComponent"), 2);
}

// ---- sh:uniqueValuesFor ----

#[test]
fn unique_values_for_single_property() {
    // Two target nodes sharing the same ex:id -> a symmetric pair of results.
    // A node with no ex:id contributes nothing. Mirrors W3C uniqueValuesFor-001.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetClass ex:Thing ;
          sh:uniqueValuesFor ex:id .
    "#;
    let data = r#"
        ex:a a ex:Thing ; ex:id "001" .
        ex:b a ex:Thing ; ex:id "002" .
        ex:dup1 a ex:Thing ; ex:id "DUP" .
        ex:dup2 a ex:Thing ; ex:id "DUP" .
        ex:noid a ex:Thing .
    "#;
    let r = run(data, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    // dup1 -> dup2 and dup2 -> dup1: exactly two results.
    assert_eq!(count_component(&r, "UniqueValuesForConstraintComponent"), 2);
}

#[test]
fn unique_values_for_composite_key_and_missing_values() {
    // Composite key ( ex:notation ex:scheme ): only nodes that agree on BOTH
    // collide. Mirrors W3C uniqueValuesFor-002.
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetClass ex:Concept ;
          sh:uniqueValuesFor ( ex:notation ex:scheme ) .
    "#;
    let data = r#"
        ex:v1 a ex:Concept ; ex:notation "A1" ; ex:scheme ex:S1 .
        ex:v2 a ex:Concept ; ex:notation "A2" ; ex:scheme ex:S1 .
        ex:bad1 a ex:Concept ; ex:notation "A1" ; ex:scheme ex:S2 .
        ex:bad2 a ex:Concept ; ex:notation "A1" ; ex:scheme ex:S2 .
    "#;
    let r = run(data, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    // Only bad1/bad2 agree on both -> two results; v1 (notation A1, scheme S1)
    // does NOT collide with bad1 (scheme differs).
    assert_eq!(count_component(&r, "UniqueValuesForConstraintComponent"), 2);
}

#[test]
fn unique_values_for_no_values_conforms() {
    // No instance has the property -> conforms (W3C uniqueValuesFor-004).
    let shapes = r#"
        ex:S a sh:NodeShape ;
          sh:targetClass ex:Thing ;
          sh:uniqueValuesFor ex:id .
    "#;
    let r = run("ex:a a ex:Thing . ex:b a ex:Thing .", shapes);
    assert!(r.conforms, "{}", r.to_text());
}

// ---- [OPUS-4.8] (sq-bif.10) genuinely-dark eval.rs dispatch branches ----
//
// The sq-qap0 datatype/pattern tests exercised only WELL-FORMED literals and a
// VALID regex. These pin the still-dark conjuncts: the `&& well_formed(l)` FALSE
// branch in `Component::Datatype` (right datatype IRI, ill-formed lexical value)
// and the `regex_for(..) == None` branch in `Component::Pattern` (an uncompilable
// pattern yields no regex — the constraint is SKIPPED with a diagnostic, sq-lz99x,
// NOT fail-closed flagging every value).

/// `sh:datatype` is NOT satisfied by a literal that carries the right datatype IRI
/// but whose lexical value is ill-formed for it — the `well_formed(l)` conjunct.
/// `"abc"^^xsd:integer` and `"5.5"^^xsd:integer` both have datatype xsd:integer yet
/// are lexically invalid; `"42"^^xsd:integer` is well-formed and conforms.
#[test]
fn datatype_rejects_ill_formed_lexical_value() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ; sh:datatype xsd:integer ] .
    "#;
    // Three values all TYPED xsd:integer: "abc" (non-numeric) and "5.5" (decimal,
    // not an integer) are ill-formed; "42" is well-formed.
    let data = r#"
        ex:n ex:v "abc"^^xsd:integer , "5.5"^^xsd:integer , "42"^^xsd:integer .
    "#;
    let r = run(data, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    // Exactly the two ill-formed values are flagged; "42" passes.
    assert_eq!(
        count_component(&r, "DatatypeConstraintComponent"),
        2,
        "{}",
        r.to_text()
    );
    assert_eq!(
        flagged_values(&r),
        vec![
            "\"5.5\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string(),
            "\"abc\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string(),
        ],
        "{}",
        r.to_text()
    );
}

/// A bounded integer datatype (`xsd:byte`) rejects an out-of-range lexical value
/// even though it is a syntactically valid integer — the range arm of
/// `well_formed`. `"300"` exceeds the signed-byte range (-128..=127).
#[test]
fn datatype_rejects_out_of_range_bounded_integer() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:v ; sh:datatype xsd:byte ] .
    "#;
    let r = run(
        r#"ex:n ex:v "300"^^xsd:byte , "100"^^xsd:byte ."#,
        shapes,
    );
    assert!(!r.conforms, "{}", r.to_text());
    // "300" is out of the byte range; "100" is in range.
    assert_eq!(count_component(&r, "DatatypeConstraintComponent"), 1);
    assert_eq!(
        flagged_values(&r),
        vec!["\"300\"^^<http://www.w3.org/2001/XMLSchema#byte>".to_string()]
    );
}

/// [OPUS-4.8] (sq-lz99x) An uncompilable `sh:pattern` (a regex that fails to
/// compile) yields NO regex, so the `Component::Pattern` `regex_for(..) == None`
/// branch SKIPS the constraint (the crate's lenient ill-formed-shape policy) and
/// records a diagnostic — it does NOT fail-closed by flagging every value. The
/// unbalanced `[` is a lexical regex error. Before sq-lz99x both values here were
/// (wrongly) reported as violations.
#[test]
fn pattern_with_invalid_regex_is_skipped_with_diagnostic() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:code ; sh:pattern "[" ] .
    "#;
    // Two values; the (only) constraint is uncompilable, so NEITHER is flagged.
    let r = run(r#"ex:n ex:code "ab" , "cd" ."#, shapes);
    assert!(
        r.conforms,
        "a skipped uncompilable pattern reports no violations: {}",
        r.to_text()
    );
    assert_eq!(
        count_component(&r, "PatternConstraintComponent"),
        0,
        "{}",
        r.to_text()
    );
    // The skip is surfaced, once, as a diagnostic naming the component.
    assert_eq!(r.diagnostics.len(), 1, "{}", r.to_text());
    let d = &r.diagnostics[0];
    assert!(
        d.source_component.ends_with("PatternConstraintComponent"),
        "{}",
        d.source_component
    );
    assert!(d.message.contains("SKIPPED"), "{}", d.message);
    // to_text surfaces the diagnostic even though the report conforms.
    assert!(r.to_text().contains("diagnostic"), "{}", r.to_text());
}

/// [OPUS-4.8] (sq-lz99x) The bead's exact repro: a negative-lookahead pattern
/// `^(?!(TODO|TBD)).*` with `sh:flags "i"`. The Rust `regex` crate (like the XML
/// Schema regex flavour the SHACL spec ties `sh:pattern` to) has NO lookahead, so
/// the pattern does not compile. Pre-fix this flagged EVERY conformant string as a
/// violation; now the constraint is SKIPPED with a diagnostic and a conformant
/// value passes. (Express such a check as a POSITIVE-match `sh:sparql` REGEX
/// constraint instead — see the bead.)
#[test]
fn pattern_lookahead_is_skipped_not_fail_closed() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:title ; sh:pattern "^(?!(TODO|TBD)).*" ; sh:flags "i" ] .
    "#;
    let r = run(r#"ex:n ex:title "A real title" ."#, shapes);
    assert!(
        r.conforms,
        "an unsupported-lookahead pattern must not fail-close a conformant value: {}",
        r.to_text()
    );
    assert_eq!(count_component(&r, "PatternConstraintComponent"), 0);
    assert_eq!(r.diagnostics.len(), 1, "{}", r.to_text());
    // The diagnostic carries the regex crate's own error (names look-around) and
    // notes the flags it was compiled with.
    let m = &r.diagnostics[0].message;
    assert!(m.contains("flags \"i\""), "{m}");
    assert!(m.to_lowercase().contains("look"), "{m}");
}

/// [OPUS-4.8] (sq-lz99x) A WELL-FORMED `sh:pattern` is unaffected by the
/// skip-on-uncompilable change: real violations are still reported and no spurious
/// diagnostic is recorded.
#[test]
fn pattern_valid_still_reports_violations_and_no_diagnostic() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property [ sh:path ex:code ; sh:pattern "^ab" ] .
    "#;
    let r = run(r#"ex:n ex:code "abc" , "xyz" ."#, shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(count_component(&r, "PatternConstraintComponent"), 1);
    assert_eq!(flagged_values(&r), vec!["\"xyz\"".to_string()]);
    assert!(r.diagnostics.is_empty(), "{}", r.to_text());
}

/// [OPUS-4.8] (sq-lz99x) A diagnostic is recorded ONCE per (shape, pattern), not
/// once per focus node or value — so a shape with many targets does not flood the
/// report.
#[test]
fn pattern_skip_diagnostic_is_deduplicated() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetClass ex:T ;
          sh:property [ sh:path ex:code ; sh:pattern "[" ] .
    "#;
    // Three focus nodes, each with two values: 6 (focus, value) pairs total.
    let data = r#"
        ex:a a ex:T ; ex:code "x" , "y" .
        ex:b a ex:T ; ex:code "p" , "q" .
        ex:c a ex:T ; ex:code "m" , "n" .
    "#;
    let r = run(data, shapes);
    assert!(r.conforms, "{}", r.to_text());
    assert_eq!(
        r.diagnostics.len(),
        1,
        "the skip is reported once, not per focus/value: {}",
        r.to_text()
    );
}

// ----------------------------------------------------------------------------
// [OPUS-4.8] (sq-pb0wm, epic sq-waf9o) Per-constraint-statement RDF-1.2
// reified-annotation overrides (SHACL 1.2 Core). A `{| … |}` annotation on ONE
// constraint statement `(<shape> P O)` overrides JUST that occurrence:
//   * `{| sh:deactivated true |}` suppresses only that constraint (the shape's
//     OTHER constraints still validate) — distinct from shape-level
//     `sh:deactivated`, which suppresses the whole shape.
//   * `{| sh:message "…" |}` sets the result message for only that constraint.
//   * `{| sh:severity sh:Warning |}` sets the result severity for only that
//     constraint's violations.
// These drive the FINAL SHACL-1.2 core gap (`misc/{deactivated-003,message-002,
// severity-003}`); the assertions below go through the REAL `validate()`. The
// `{| |}` form is parsed by oxttl's rdf-12 Turtle support (no extra wiring).
// ----------------------------------------------------------------------------

const SH_WARNING: &str = "http://www.w3.org/ns/shacl#Warning";
const SH_VIOLATION: &str = "http://www.w3.org/ns/shacl#Violation";

/// `{| sh:deactivated true |}` on a single `sh:datatype` statement suppresses
/// ONLY that constraint occurrence — mirrors `misc/deactivated-003`'s datatype
/// half: the shape conforms despite the value violating the (now-deactivated)
/// datatype constraint.
#[test]
fn per_statement_deactivate_suppresses_only_that_constraint() {
    // The focus is a string literal target node that is NOT an xsd:boolean, so
    // the datatype constraint WOULD fire — but the per-statement annotation
    // deactivates this occurrence, so the shape conforms.
    let data = r#" ex:x ex:dummy "irrelevant" . "#;
    let annotated = r#"
        ex:S a sh:NodeShape ; sh:targetNode "hello" ;
          sh:datatype xsd:boolean {| sh:deactivated true |} .
    "#;
    let r = run(data, annotated);
    assert!(
        r.conforms,
        "per-statement-deactivated datatype must produce NO result: {}",
        r.to_text()
    );
    // Control: WITHOUT the `{| … |}` annotation the SAME constraint fires.
    let live = r#"
        ex:S a sh:NodeShape ; sh:targetNode "hello" ;
          sh:datatype xsd:boolean .
    "#;
    let r2 = run(data, live);
    assert!(
        !r2.conforms,
        "control (no annotation) must violate: {}",
        r2.to_text()
    );
    assert_eq!(count_component(&r2, "DatatypeConstraintComponent"), 1);
}

/// A per-statement `{| sh:deactivated true |}` on ONE constraint must NOT
/// suppress the shape's OTHER constraints (the occurrence-scoped semantics — not
/// shape-level deactivation). The deactivated `sh:datatype` is silent while the
/// live `sh:minLength` still fires.
#[test]
fn per_statement_deactivate_leaves_sibling_constraints_live() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode "hi" ;
          sh:datatype xsd:boolean {| sh:deactivated true |} ;
          sh:minLength 5 .
    "#;
    let r = run("ex:x ex:p ex:o .", shapes);
    assert!(!r.conforms, "the live sh:minLength must still fire: {}", r.to_text());
    assert_eq!(
        count_component(&r, "DatatypeConstraintComponent"),
        0,
        "the per-statement-deactivated datatype must be silent: {}",
        r.to_text()
    );
    assert_eq!(
        count_component(&r, "MinLengthConstraintComponent"),
        1,
        "the un-annotated sibling constraint must still report: {}",
        r.to_text()
    );
}

/// `{| sh:deactivated true |}` on a single `sh:property` statement suppresses
/// only that property-constraint occurrence (the nested shape is not validated) —
/// the `sh:property` half of `misc/deactivated-003`.
#[test]
fn per_statement_deactivate_on_property_constraint() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property ex:P {| sh:deactivated true |} .
        ex:P a sh:PropertyShape ; sh:path ex:q ; sh:minCount 1 .
    "#;
    // ex:n has NO ex:q value, so the (live) minCount would fire — but the
    // property statement is deactivated, so the shape conforms.
    let r = run("ex:n ex:other ex:o .", shapes);
    assert!(
        r.conforms,
        "deactivated sh:property must not validate the nested shape: {}",
        r.to_text()
    );
    // Control: without the annotation the nested minCount fires.
    let live = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property ex:P .
        ex:P a sh:PropertyShape ; sh:path ex:q ; sh:minCount 1 .
    "#;
    let r2 = run("ex:n ex:other ex:o .", live);
    assert!(!r2.conforms, "control must violate minCount: {}", r2.to_text());
}

/// `{| sh:message "…"@en |}` on a single constraint statement sets the result
/// message for ONLY that occurrence's violations (overriding the absence of a
/// shape-level message) — mirrors `misc/message-002`.
#[test]
fn per_statement_message_override_applies_to_that_constraint() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:InvalidNode ;
          sh:datatype xsd:integer {| sh:message "Test message"@en |} .
    "#;
    let r = run("ex:InvalidNode ex:p ex:o .", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    let msgs = r.results[0].effective_messages();
    let want = Term::Literal(oxrdf::Literal::new_language_tagged_literal_unchecked(
        "Test message",
        "en",
    ));
    assert!(
        msgs.contains(&want),
        "result message must be the per-statement override, got {msgs:?}"
    );
}

/// `{| sh:severity sh:Warning |}` on a single constraint statement sets the
/// result severity for ONLY that occurrence's violations (the default is
/// sh:Violation) — mirrors `misc/severity-003`.
#[test]
fn per_statement_severity_override_applies_to_that_constraint() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode "Hello" ;
          sh:datatype xsd:integer {| sh:severity sh:Warning |} .
    "#;
    let r = run("ex:x ex:p ex:o .", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert_eq!(
        r.results[0].severity, SH_WARNING,
        "the per-statement severity override must apply: {}",
        r.to_text()
    );
}

/// A per-statement override applies to ONLY the annotated occurrence: with two
/// constraints on one shape, only the one annotated with `sh:severity sh:Warning`
/// gets the Warning severity; the other keeps the default sh:Violation.
#[test]
fn per_statement_severity_is_occurrence_scoped() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode "Hello" ;
          sh:datatype xsd:integer {| sh:severity sh:Warning |} ;
          sh:minLength 99 .
    "#;
    let r = run("ex:x ex:p ex:o .", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 2, "{}", r.to_text());
    let dt = r
        .results
        .iter()
        .find(|x| x.source_component.ends_with("DatatypeConstraintComponent"))
        .expect("datatype result");
    let ml = r
        .results
        .iter()
        .find(|x| x.source_component.ends_with("MinLengthConstraintComponent"))
        .expect("minLength result");
    assert_eq!(dt.severity, SH_WARNING, "annotated occurrence → Warning");
    assert_eq!(
        ml.severity, SH_VIOLATION,
        "un-annotated occurrence keeps the default Violation"
    );
}

/// [SONNET-4.6] (sq-7os4t) The two qualified-count parameters are distinct
/// causing statements even though the parser combines them into one component.
/// Their annotations must therefore be selected independently for each result.
#[test]
fn per_statement_overrides_apply_to_qualified_count_causing_statement() {
    let shapes = r#"
        ex:S a sh:PropertyShape ; sh:targetNode ex:n ; sh:path ex:p ;
          sh:qualifiedValueShape [ sh:nodeKind sh:IRI ] ;
          sh:qualifiedMinCount 2 {| sh:severity sh:Warning |} ;
          sh:qualifiedMaxCount 0 {| sh:message "qualified maximum" |} .
    "#;
    let r = run("ex:n ex:p ex:value .", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 2, "{}", r.to_text());

    let min = r
        .results
        .iter()
        .find(|x| {
            x.source_component
                .ends_with("QualifiedMinCountConstraintComponent")
        })
        .expect("qualified minimum result");
    assert_eq!(min.severity, SH_WARNING);

    let max = r
        .results
        .iter()
        .find(|x| {
            x.source_component
                .ends_with("QualifiedMaxCountConstraintComponent")
        })
        .expect("qualified maximum result");
    assert_eq!(max.severity, SH_VIOLATION);
    assert!(
        max.effective_messages()
            .iter()
            .any(|m| matches!(m, Term::Literal(l) if l.value() == "qualified maximum")),
        "qualified maximum result must use its statement annotation: {}",
        r.to_text()
    );
}

// ----------------------------------------------------------------------------
// [FABLE-5] (sq-1jemy) Per-statement overrides on RECURSING components. A
// `{| sh:message … |}` / `{| sh:severity … |}` on a shape-referencing constraint
// statement (`sh:node` / `sh:not`) governs the COMPOSITE component's OWN result
// (the SHACL-1.2 severity precedence keys on the reifier of "the triples
// containing the parameters of the constraint that caused the result"). Before
// this fix the nested `conforms()` / `validate_shape()` recursion reset
// `active_meta` to `None` before the composite arm's `result()` read it, so the
// override was silently dropped. Conversely, the override does NOT govern the
// NESTED shape's results (they are caused by the nested shape's own constraint
// statements) — the reading recorded in research/shacl12-conformance-gap.md §6.
// ----------------------------------------------------------------------------

/// `{| sh:severity sh:Warning |}` on a `sh:node` statement survives the nested
/// `conforms()` recursion: the NodeConstraintComponent's own result carries the
/// override. Term-level route (literal focus, no path ⇒ `ValueNodes::Terms`).
#[test]
fn per_statement_severity_override_survives_sh_node_recursion() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode "hello" ;
          sh:node ex:N {| sh:severity sh:Warning |} .
        ex:N a sh:NodeShape ; sh:nodeKind sh:IRI .
    "#;
    let r = run("ex:x ex:p ex:o .", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(count_component(&r, "NodeConstraintComponent"), 1, "{}", r.to_text());
    let node_res = r
        .results
        .iter()
        .find(|x| x.source_component.ends_with("NodeConstraintComponent"))
        .expect("node result");
    assert_eq!(
        node_res.severity, SH_WARNING,
        "the sh:node occurrence's severity override must survive the nested \
         shape evaluation: {}",
        r.to_text()
    );
}

/// The `sh:node` override on the ID-FAST route (pathed property shape ⇒ value
/// nodes stay dictionary ids and the id-level `Component::Node` arm calls
/// `conforms_id`). Same invariant as the Term-level twin above.
#[test]
fn per_statement_severity_override_survives_sh_node_recursion_idfast() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:property ex:P .
        ex:P a sh:PropertyShape ; sh:path ex:q ;
          sh:node ex:N {| sh:severity sh:Warning |} .
        ex:N a sh:NodeShape ; sh:datatype xsd:integer .
    "#;
    // ex:v is an IRI in the data graph (id-resolvable), not an xsd:integer.
    let r = run("ex:n ex:q ex:v .", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(count_component(&r, "NodeConstraintComponent"), 1, "{}", r.to_text());
    let node_res = r
        .results
        .iter()
        .find(|x| x.source_component.ends_with("NodeConstraintComponent"))
        .expect("node result");
    assert_eq!(
        node_res.severity, SH_WARNING,
        "the id-level sh:node arm must also see the surviving override: {}",
        r.to_text()
    );
}

/// `{| sh:message … |}` on a `sh:node` statement: the composite result's
/// message is the per-statement override, not the (absent) shape-level one.
#[test]
fn per_statement_message_override_survives_sh_node_recursion() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode "hello" ;
          sh:node ex:N {| sh:message "node override"@en |} .
        ex:N a sh:NodeShape ; sh:nodeKind sh:IRI .
    "#;
    let r = run("ex:x ex:p ex:o .", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    let node_res = r
        .results
        .iter()
        .find(|x| x.source_component.ends_with("NodeConstraintComponent"))
        .expect("node result");
    let want = Term::Literal(oxrdf::Literal::new_language_tagged_literal_unchecked(
        "node override",
        "en",
    ));
    assert!(
        node_res.effective_messages().contains(&want),
        "the sh:node occurrence's message override must survive the nested \
         shape evaluation, got {:?}",
        node_res.effective_messages()
    );
}

/// `{| sh:severity sh:Warning |}` on a `sh:not` statement (the other
/// single-statement recursing composite) survives its `conforms()` recursion.
#[test]
fn per_statement_severity_override_survives_sh_not_recursion() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:not ex:N {| sh:severity sh:Warning |} .
        ex:N a sh:NodeShape ; sh:nodeKind sh:IRI .
    "#;
    // ex:n IS an IRI, so it conforms to the negated shape → sh:not fires.
    let r = run("ex:n ex:p ex:o .", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(count_component(&r, "NotConstraintComponent"), 1, "{}", r.to_text());
    let not_res = r
        .results
        .iter()
        .find(|x| x.source_component.ends_with("NotConstraintComponent"))
        .expect("not result");
    assert_eq!(
        not_res.severity, SH_WARNING,
        "the sh:not occurrence's severity override must survive the nested \
         shape evaluation: {}",
        r.to_text()
    );
}

/// Occurrence scoping still holds around a recursing component: the annotated
/// `sh:node` result carries the override while an un-annotated SIBLING
/// constraint evaluated AFTER it keeps the default sh:Violation (the restore
/// must not bleed the override into later loop iterations).
#[test]
fn per_statement_override_on_sh_node_is_occurrence_scoped() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode "hello" ;
          sh:node ex:N {| sh:severity sh:Warning |} ;
          sh:minLength 99 .
        ex:N a sh:NodeShape ; sh:nodeKind sh:IRI .
    "#;
    let r = run("ex:x ex:p ex:o .", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 2, "{}", r.to_text());
    let node_res = r
        .results
        .iter()
        .find(|x| x.source_component.ends_with("NodeConstraintComponent"))
        .expect("node result");
    let ml = r
        .results
        .iter()
        .find(|x| x.source_component.ends_with("MinLengthConstraintComponent"))
        .expect("minLength result");
    assert_eq!(node_res.severity, SH_WARNING, "annotated sh:node → Warning");
    assert_eq!(
        ml.severity, SH_VIOLATION,
        "un-annotated sibling keeps the default Violation: {}",
        r.to_text()
    );
}

/// A `{| sh:severity |}` on a `sh:property` statement does NOT govern the
/// NESTED property shape's results: those are caused by the nested shape's own
/// constraint statements (here its `sh:minCount`), whose reifiers — not the
/// outer `sh:property` statement's — drive the severity precedence. Deactivation
/// on `sh:property` (the pre-recursion skip) is covered above; message/severity
/// on it have no composite result to govern in this implementation.
#[test]
fn per_statement_override_on_sh_property_does_not_govern_nested_results() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:n ;
          sh:property ex:P {| sh:severity sh:Warning |} .
        ex:P a sh:PropertyShape ; sh:path ex:q ; sh:minCount 1 .
    "#;
    let r = run("ex:n ex:other ex:o .", shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(count_component(&r, "MinCountConstraintComponent"), 1, "{}", r.to_text());
    assert_eq!(
        r.results[0].severity, SH_VIOLATION,
        "the nested shape's result keeps the nested shape's own severity — the \
         outer sh:property annotation must not leak in: {}",
        r.to_text()
    );
}

/// #3526: `sh:in` and `sh:hasValue` are RDF-term membership (SHACL 1.0 §4.8.1–4.8.2).
/// A distinct literal with the same value is not a member, for numerics and for two
/// dateTime lexicals that denote one instant.
#[test]
fn in_and_has_value_use_rdf_term_membership() {
    let shapes = r#"
        ex:S a sh:NodeShape ; sh:targetNode ex:a ;
            sh:property [ sh:path ex:n ; sh:in ( 1 ) ] ;
            sh:property [ sh:path ex:t ; sh:hasValue "2000-01-01T00:00:59.99999999999999999999Z"^^xsd:dateTime ] .
    "#;
    let data = r#"
        ex:a ex:n "01"^^xsd:integer ;
             ex:t "2000-01-01T00:00:59.999999999999999999990Z"^^xsd:dateTime .
    "#;
    let r = run(data, shapes);
    assert_eq!(count_component(&r, "InConstraintComponent"), 1, "{r:?}");
    assert_eq!(count_component(&r, "HasValueConstraintComponent"), 1, "{r:?}");
    // The identical terms are members.
    let same = r#"
        ex:a ex:n 1 ;
             ex:t "2000-01-01T00:00:59.99999999999999999999Z"^^xsd:dateTime .
    "#;
    assert!(run(same, shapes).conforms);
}

// ---- every comparison constraint over every pair of temporal families ----

/// One value per temporal family: a date, a dateTime, a dateTimeStamp (the dateTime
/// family), and a time. The earlier of each same-family pair is first.
const TEMPORALS: [(&str, &str); 4] = [
    ("date", "\"2000-01-01\"^^xsd:date"),
    ("dateTime", "\"2000-01-01T00:00:00Z\"^^xsd:dateTime"),
    ("dateTimeStamp", "\"2000-01-02T00:00:00Z\"^^xsd:dateTimeStamp"),
    ("time", "\"01:00:00Z\"^^xsd:time"),
];

fn family(name: &str) -> &str {
    if name == "dateTimeStamp" {
        "dateTime"
    } else {
        name
    }
}

/// `value` against `other` under each comparison constraint; `true` = conforms.
fn comparisons(value: &str, other: &str) -> [(&'static str, bool); 6] {
    let pair = |c: &str| {
        let shapes = format!(
            "ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:property [ sh:path ex:v ; sh:{c} ex:w ] ."
        );
        run(&format!("ex:n ex:v {value} ; ex:w {other} ."), &shapes).conforms
    };
    let range = |c: &str| {
        let shapes = format!(
            "ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:property [ sh:path ex:v ; sh:{c} {other} ] ."
        );
        run(&format!("ex:n ex:v {value} ."), &shapes).conforms
    };
    [
        ("lessThan", pair("lessThan")),
        ("lessThanOrEquals", pair("lessThanOrEquals")),
        ("minInclusive", range("minInclusive")),
        ("maxInclusive", range("maxInclusive")),
        ("minExclusive", range("minExclusive")),
        ("maxExclusive", range("maxExclusive")),
    ]
}

/// A temporal compared with another family is incomparable, so every comparison
/// constraint reports it (SHACL 1.0 §4.5, §4.4): a date against a dateTime, and either
/// against an xsd:time (which the old fallback anchored to 2000-01-01).
#[test]
fn every_comparison_rejects_every_mixed_temporal_pair() {
    for (na, a) in TEMPORALS {
        for (nb, b) in TEMPORALS {
            if family(na) == family(nb) {
                continue;
            }
            for (c, conforms) in comparisons(a, b) {
                assert!(!conforms, "{c}: {na} {a} vs {nb} {b} must be incomparable");
            }
        }
    }
}

/// The control: a same-family pair still compares, both ways round.
#[test]
fn every_comparison_orders_a_same_family_temporal_pair() {
    let (_, dt) = TEMPORALS[1];
    let (_, stamp) = TEMPORALS[2];
    let earlier_than_later = [true, true, false, true, false, true];
    for ((c, got), want) in comparisons(dt, stamp).into_iter().zip(earlier_than_later) {
        assert_eq!(got, want, "{c}: dateTime vs later dateTimeStamp");
    }
    let times = comparisons("\"00:30:00Z\"^^xsd:time", "\"24:00:00Z\"^^xsd:time");
    // 24:00:00 is 00:00:00, so 00:30 is after it.
    let later_than_earlier = [false, false, true, false, true, false];
    for ((c, got), want) in times.into_iter().zip(later_than_earlier) {
        assert_eq!(got, want, "{c}: 00:30 vs 24:00 times");
    }
}

/// An invalid xsd:time lexical compares with nothing, so every comparison constraint
/// reports it. Hour 24 is valid only as 24:00:00 with a zero fraction, and is midnight.
#[test]
fn only_a_valid_time_lexical_compares() {
    let later_than_earlier = [false, false, true, false, true, false];
    let zero = comparisons("\"00:30:00Z\"^^xsd:time", "\"24:00:00.000Z\"^^xsd:time");
    for ((c, got), want) in zero.into_iter().zip(later_than_earlier) {
        assert_eq!(got, want, "{c}: 00:30 vs 24:00:00.000");
    }
    for bad in [
        "24:00:00.5Z",
        "24:00:00.0.0",
        "24:00:00.0.5Z",
        "24:00:00.0Z.5",
        "24:00:01",
        "24:01:00",
        "23:59:60",
        "12:00Z",
        "12:00:00+14:01",
        "12:00:00+15:00",
        "12:00:00+1:00",
        "12:00:00Z+01:00",
        "24:00:00.0+01:00:00",
    ] {
        let bad = format!("\"{bad}\"^^xsd:time");
        for (value, other) in [("\"00:30:00Z\"^^xsd:time", bad.as_str()), (bad.as_str(), "\"00:30:00Z\"^^xsd:time")] {
            for (c, conforms) in comparisons(value, other) {
                assert!(!conforms, "{c}: {value} vs {other} must be incomparable");
            }
        }
    }
}
