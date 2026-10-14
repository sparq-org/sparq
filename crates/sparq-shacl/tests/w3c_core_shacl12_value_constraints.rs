//! (sq-sx15d, epic sq-waf9o) SHACL 1.2 CORE value-constraint coverage.
//!
//! Direct, fixture-free tests for the SHACL-1.2 value constraints implemented in
//! sq-sx15d, mirroring the W3C `shacl12-test-suite` `core/{property,misc,
//! validation-reports}` entries this bead targets. Each test inlines the suite
//! entry's shapes+data (the fetched suite tree is gitignored, so the data is
//! copied here so the test runs on a fresh checkout) and asserts the produced
//! report against the suite's expected report through the REAL `validate()`:
//!
//!   * A. path-valued comparands — `equals-002`, `disjoint-002`, `lessThan-003`,
//!     `lessThanOrEquals-002` (`sh:equals` / `sh:disjoint` / `sh:lessThan` /
//!     `sh:lessThanOrEquals` taking a SHACL property PATH, often an RDF-list
//!     sequence, as comparand).
//!   * B. disjunctive `sh:class` — `class-002` (`sh:class ( ex:A ex:B )`).
//!   * C. four new components — `subsetOf-001`, `someValue-001`,
//!     `singleLine-001`, `rootClass-001`.
//!   * D. severity-threshold conformance — `severity-004` (sh:Debug),
//!     `severity-005` (sh:Trace), `conformance-disallows-001` (a Warning result
//!     conforming under a custom `sh:conformanceDisallows sh:Violation`).
//!
//! The suite's own manifest-driven runner lives in `w3c_core_shacl12.rs` (which a
//! parallel PR extends to walk `core/property|misc|validation-reports`); this
//! file is the in-tree, suite-independent counterpart for the same entries.

use oxrdf::Term;
use sparq_core::Graph;
use sparq_shacl::{validate, Path, ValidationReport, ValidationResult};

const PREFIXES: &str = r#"
@prefix ex: <http://example.com/ns#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

const NS: &str = "http://example.com/ns#";
const SH: &str = "http://www.w3.org/ns/shacl#";

/// Validates one document acting as BOTH the data and the shapes graph (the
/// suite entries set `sht:dataGraph` = `sht:shapesGraph` = the test document).
fn run(doc: &str) -> ValidationReport {
    let g = Graph::load_str(&format!("{PREFIXES}{doc}"), "turtle").unwrap();
    validate(&g, &g)
}

fn ex(local: &str) -> Term {
    Term::NamedNode(oxrdf::NamedNode::new_unchecked(format!("{NS}{local}")))
}

fn sh_comp(local: &str) -> String {
    format!("{SH}{local}")
}

/// An expected validation result: focus / path / value / component / severity.
/// `None` for `value` means "no `sh:value`"; `Some(None)` is not expressible —
/// use `value_none` for the no-value case.
struct Exp {
    focus: Term,
    path: Option<Path>,
    value: Option<Term>,
    component: String,
    severity: String,
}

fn pred_path(local: &str) -> Path {
    Path::Predicate(format!("{NS}{local}"))
}

fn violation(focus: Term, path: Option<Path>, value: Option<Term>, component: &str) -> Exp {
    Exp {
        focus,
        path,
        value,
        component: sh_comp(component),
        severity: format!("{SH}Violation"),
    }
}

/// Asserts the report's results are a 1:1 match for `expected` (multiset, with
/// blank nodes matching any blank node), and that `conforms` matches.
fn assert_report(r: &ValidationReport, conforms: bool, expected: &[Exp]) {
    assert_eq!(
        r.conforms,
        conforms,
        "conforms mismatch: got {} results: {}",
        r.results.len(),
        summarize(&r.results)
    );
    assert_eq!(
        expected.len(),
        r.results.len(),
        "result count: expected {}, got {}: {}",
        expected.len(),
        r.results.len(),
        summarize(&r.results)
    );
    let mut used = vec![false; r.results.len()];
    for e in expected {
        let found = r
            .results
            .iter()
            .enumerate()
            .position(|(j, a)| !used[j] && matches(e, a));
        match found {
            Some(j) => used[j] = true,
            None => panic!(
                "no actual result matches expected [focus {} comp {} value {:?}]; got {}",
                e.focus,
                e.component.rsplit('#').next().unwrap_or(""),
                e.value.as_ref().map(|v| v.to_string()),
                summarize(&r.results)
            ),
        }
    }
}

fn matches(e: &Exp, a: &ValidationResult) -> bool {
    if !term_matches(&e.focus, &a.focus_node) {
        return false;
    }
    if e.path != a.path {
        return false;
    }
    match (&e.value, &a.value) {
        (None, None) => {}
        (Some(ev), Some(av)) if term_matches(ev, av) => {}
        _ => return false,
    }
    e.component == a.source_component && e.severity == a.severity
}

fn term_matches(e: &Term, a: &Term) -> bool {
    match (e, a) {
        (Term::BlankNode(_), Term::BlankNode(_)) => true,
        _ => e == a,
    }
}

fn summarize(rs: &[ValidationResult]) -> String {
    rs.iter()
        .map(|r| {
            format!(
                "[focus {} comp {} path {:?} value {} sev {}]",
                r.focus_node,
                r.source_component.rsplit('#').next().unwrap_or(""),
                r.path.as_ref().map(|p| p.to_turtle()),
                r.value.as_ref().map(|v| v.to_string()).unwrap_or_default(),
                r.severity.rsplit('#').next().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// An `xsd:integer` literal term (the suite writes bare integers like `4`).
fn int(n: i64) -> Term {
    Term::Literal(oxrdf::Literal::new_typed_literal(
        n.to_string(),
        oxrdf::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
    ))
}

fn lit(s: &str) -> Term {
    Term::Literal(oxrdf::Literal::new_simple_literal(s))
}

// ============================ A. path-valued comparands ====================

/// `equals-002`: `sh:equals ( ex:property2 ex:property3 )` — a sequence-PATH
/// comparand. Value set of ex:property1 must equal the value set reached along
/// the sequence. Five results across four invalid resources (both directions).
#[test]
fn equals_002_path_valued_comparand() {
    let doc = r#"
ex:InvalidResource1 ex:property1 "A" ; ex:property2 [ ex:property3 "B" ] .
ex:InvalidResource2 ex:property1 "A" .
ex:InvalidResource3 ex:property2 [ ex:property3 "A" ] .
ex:InvalidResource4 ex:property1 "A" ; ex:property1 "B" ; ex:property2 [ ex:property3 "A" ] .
ex:TestShape a sh:NodeShape ; sh:property ex:TestShape-property1 ;
  sh:targetNode ex:InvalidResource1, ex:InvalidResource2, ex:InvalidResource3,
                ex:InvalidResource4, ex:ValidResource1, ex:ValidResource2 .
ex:TestShape-property1 sh:path ex:property1 ; sh:equals ( ex:property2 ex:property3 ) .
ex:ValidResource1 ex:property1 "A" ; ex:property2 [ ex:property3 "A" ] .
ex:ValidResource2 ex:property1 "A" ; ex:property1 "B" ;
  ex:property2 [ ex:property3 "A" ] ; ex:property2 [ ex:property3 "B" ] .
"#;
    let r = run(doc);
    let p1 = Some(pred_path("property1"));
    assert_report(
        &r,
        false,
        &[
            violation(ex("InvalidResource1"), p1.clone(), Some(lit("A")), "EqualsConstraintComponent"),
            violation(ex("InvalidResource1"), p1.clone(), Some(lit("B")), "EqualsConstraintComponent"),
            violation(ex("InvalidResource2"), p1.clone(), Some(lit("A")), "EqualsConstraintComponent"),
            violation(ex("InvalidResource3"), p1.clone(), Some(lit("A")), "EqualsConstraintComponent"),
            violation(ex("InvalidResource4"), p1, Some(lit("B")), "EqualsConstraintComponent"),
        ],
    );
}

/// `disjoint-002`: BOTH the shape path AND the comparand are sequence paths
/// (`sh:path ( ex:property1 ex:property2 )`, `sh:disjoint ( ex:property3
/// ex:property4 )`). The reported `sh:resultPath` is the sequence path.
#[test]
fn disjoint_002_sequence_path_and_comparand() {
    let doc = r#"
ex:InvalidResource1 ex:property1 [ ex:property2 "A" ] ; ex:property3 [ ex:property4 "A" ] .
ex:InvalidResource2 ex:property1 [ ex:property2 "A" ] ; ex:property1 [ ex:property2 "B" ] ;
  ex:property3 [ ex:property4 "A" ] .
ex:TestShape a sh:NodeShape ; sh:property ex:TestShape-property1 ;
  sh:targetNode ex:InvalidResource1, ex:InvalidResource2, ex:ValidResource1, ex:ValidResource2 .
ex:TestShape-property1 sh:path ( ex:property1 ex:property2 ) ;
  sh:disjoint ( ex:property3 ex:property4 ) .
ex:ValidResource1 ex:property1 [ ex:property2 "A" ] ; ex:property3 [ ex:property4 "B" ] .
ex:ValidResource2 ex:property1 [ ex:property2 "A" ] ; ex:property1 [ ex:property2 "B" ] ;
  ex:property3 [ ex:property4 "C" ] ; ex:property3 [ ex:property4 "D" ] .
"#;
    let r = run(doc);
    let seq = Some(Path::Sequence(vec![pred_path("property1"), pred_path("property2")]));
    assert_report(
        &r,
        false,
        &[
            violation(ex("InvalidResource1"), seq.clone(), Some(lit("A")), "DisjointConstraintComponent"),
            violation(ex("InvalidResource2"), seq, Some(lit("A")), "DisjointConstraintComponent"),
        ],
    );
}

/// `lessThan-003`: `sh:lessThan ( ex:property2 ex:property3 )` — sequence-PATH
/// comparand. One result per failing (value, comparand) pair.
#[test]
fn less_than_003_path_valued_comparand() {
    let doc = r#"
ex:InvalidResource1 ex:property1 4 ; ex:property2 [ ex:property3 4 ] .
ex:InvalidResource2 ex:property1 4 ; ex:property1 6 ; ex:property2 [ ex:property3 5 ] .
ex:InvalidResource3 ex:property1 5 ; ex:property2 [ ex:property3 4 ] .
ex:TestShape a sh:NodeShape ; sh:property ex:TestShape-property1 ;
  sh:targetNode ex:InvalidResource1, ex:InvalidResource2, ex:InvalidResource3,
                ex:ValidResource1, ex:ValidResource2 .
ex:TestShape-property1 sh:path ex:property1 ; sh:lessThan ( ex:property2 ex:property3 ) .
ex:ValidResource1 ex:property1 4 ; ex:property2 [ ex:property3 6 ] .
ex:ValidResource2 ex:property1 3.1 .
"#;
    let r = run(doc);
    let p1 = Some(pred_path("property1"));
    assert_report(
        &r,
        false,
        &[
            violation(ex("InvalidResource1"), p1.clone(), Some(int(4)), "LessThanConstraintComponent"),
            violation(ex("InvalidResource2"), p1.clone(), Some(int(6)), "LessThanConstraintComponent"),
            violation(ex("InvalidResource3"), p1, Some(int(5)), "LessThanConstraintComponent"),
        ],
    );
}

/// `lessThanOrEquals-002`: `sh:lessThanOrEquals ( ex:property2 ex:property3 )`.
#[test]
fn less_than_or_equals_002_path_valued_comparand() {
    let doc = r#"
ex:InvalidResource1 ex:property1 5 ; ex:property2 [ ex:property3 4 ] .
ex:InvalidResource2 ex:property1 4 ; ex:property1 6 ; ex:property2 [ ex:property3 5 ] .
ex:TestShape a sh:NodeShape ; sh:property ex:TestShape-property1 ;
  sh:targetNode ex:InvalidResource1, ex:InvalidResource2,
                ex:ValidResource1, ex:ValidResource2, ex:ValidResource3 .
ex:TestShape-property1 sh:path ex:property1 ; sh:lessThanOrEquals ( ex:property2 ex:property3 ) .
ex:ValidResource1 ex:property1 4 ; ex:property2 [ ex:property3 6 ] .
ex:ValidResource2 ex:property1 3.1 .
ex:ValidResource3 ex:property1 5 ; ex:property2 [ ex:property3 5 ] .
"#;
    let r = run(doc);
    let p1 = Some(pred_path("property1"));
    assert_report(
        &r,
        false,
        &[
            violation(ex("InvalidResource1"), p1.clone(), Some(int(5)), "LessThanOrEqualsConstraintComponent"),
            violation(ex("InvalidResource2"), p1, Some(int(6)), "LessThanOrEqualsConstraintComponent"),
        ],
    );
}

/// A bare-predicate comparand (the SHACL-1.0 `-001` form) still works after the
/// switch from a `String` predicate to a `Path` comparand — backward-compat.
#[test]
fn equals_predicate_comparand_backward_compatible() {
    let doc = r#"
ex:n a ex:T ; ex:a 1, 2 ; ex:b 2, 3 .
ex:S a sh:NodeShape ; sh:targetNode ex:n ;
  sh:property [ sh:path ex:a ; sh:equals ex:b ] .
"#;
    let r = run(doc);
    // ex:a = {1,2}, ex:b = {2,3}: 1 absent from b, 3 absent from a -> 2 results.
    assert_eq!(
        r.results
            .iter()
            .filter(|x| x.source_component.ends_with("EqualsConstraintComponent"))
            .count(),
        2,
        "{}",
        summarize(&r.results)
    );
    assert!(!r.conforms);
}

// ============================ B. disjunctive sh:class ======================

/// `class-002`: `sh:class ( ex:SuperClass ex:OtherClass )` — instance of ANY
/// listed class, subclass-aware (ex:SubClass ⊑ ex:SuperClass conforms).
#[test]
fn class_002_disjunctive_class_list() {
    let doc = r#"
ex:InvalidResource1 rdf:type rdfs:Resource ;
  ex:testProperty ex:InvalidResource1 ; ex:testProperty "A string" .
ex:OtherClass rdf:type rdfs:Class .
ex:OtherClassInstance rdf:type ex:OtherClass .
ex:SubClass rdf:type rdfs:Class ; rdfs:subClassOf ex:SuperClass .
ex:SubClassInstance rdf:type ex:SubClass .
ex:SuperClass rdf:type rdfs:Class .
ex:SuperClassInstance rdf:type ex:SuperClass .
ex:TestShape a sh:NodeShape ; sh:property ex:TestShape-testProperty ;
  sh:targetNode ex:InvalidResource1, ex:ValidResource1, ex:ValidResource2 .
ex:TestShape-testProperty sh:path ex:testProperty ;
  sh:class ( ex:SuperClass ex:OtherClass ) .
ex:ValidResource1 rdf:type rdfs:Resource ;
  ex:testProperty ex:OtherClassInstance, ex:SubClassInstance, ex:SuperClassInstance .
ex:ValidResource2 rdf:type rdfs:Resource ;
  ex:testProperty [ rdf:type ex:SubClass ] ; ex:testProperty [ rdf:type ex:SuperClass ] .
"#;
    let r = run(doc);
    let tp = Some(pred_path("testProperty"));
    assert_report(
        &r,
        false,
        &[
            violation(ex("InvalidResource1"), tp.clone(), Some(ex("InvalidResource1")), "ClassConstraintComponent"),
            violation(ex("InvalidResource1"), tp, Some(lit("A string")), "ClassConstraintComponent"),
        ],
    );
}

// ============================ C. four new components =======================

/// `subsetOf-001`: `sh:subsetOf ex:property2` — the value set of ex:property1
/// must be a SUBSET of the value set of ex:property2. One result per path value
/// absent from the comparand set.
#[test]
fn subset_of_001() {
    let doc = r#"
ex:InvalidResource1 ex:property1 "A" ; ex:property2 "B" .
ex:InvalidResource2 ex:property1 "A" .
ex:InvalidResource3 ex:property1 "A" ; ex:property1 "B" ; ex:property2 "A" .
ex:TestShape a sh:NodeShape ; sh:property ex:TestShape-property1 ;
  sh:targetNode ex:InvalidResource1, ex:InvalidResource2, ex:InvalidResource3,
                ex:ValidResource1, ex:ValidResource2 .
ex:TestShape-property1 sh:path ex:property1 ; sh:subsetOf ex:property2 .
ex:ValidResource1 ex:property1 "A" ; ex:property2 "A" .
ex:ValidResource2 ex:property1 "A" ; ex:property2 "A" ; ex:property2 "B" .
ex:property1 rdf:type rdf:Property .
ex:property2 rdf:type rdf:Property .
"#;
    let r = run(doc);
    let p1 = Some(pred_path("property1"));
    assert_report(
        &r,
        false,
        &[
            violation(ex("InvalidResource1"), p1.clone(), Some(lit("A")), "SubsetOfConstraintComponent"),
            violation(ex("InvalidResource2"), p1.clone(), Some(lit("A")), "SubsetOfConstraintComponent"),
            violation(ex("InvalidResource3"), p1, Some(lit("B")), "SubsetOfConstraintComponent"),
        ],
    );
}

/// `someValue-001`: `sh:someValue [ sh:class ex:Duck ]` — EXISTENTIAL. Alice
/// tends only a Cow -> one violation (no `sh:value`); Bob tends a Duck -> ok.
#[test]
fn some_value_001() {
    let doc = r#"
ex:Donald a ex:Duck .
ex:Daisy a ex:Duck .
ex:Eliza a ex:Cow .
ex:Alice ex:tendsAnimal ex:Eliza .
ex:Bob ex:tendsAnimal ex:Donald, ex:Eliza .
ex:DuckFarmerShape a sh:NodeShape ; sh:targetNode ex:Alice, ex:Bob ;
  sh:property ex:DuckFarmerShape-tendsAnimal .
ex:DuckFarmerShape-tendsAnimal sh:path ex:tendsAnimal ;
  sh:someValue [ sh:class ex:Duck ] .
"#;
    let r = run(doc);
    assert_report(
        &r,
        false,
        &[violation(
            ex("Alice"),
            Some(pred_path("tendsAnimal")),
            None,
            "SomeValueConstraintComponent",
        )],
    );
}

/// `singleLine-001`: `sh:singleLine true` flags strings containing LF/CR/FF/VT;
/// `sh:singleLine false` imposes no constraint (the multi-line comment is ok).
#[test]
fn single_line_001() {
    let doc = "
ex:InvalidInstanceF a ex:SingleLineExampleShape ; rdfs:label \"Invalid\\u000Cinstance\" .
ex:InvalidInstanceN a ex:SingleLineExampleShape ; rdfs:label \"Invalid\\u000Ainstance\" .
ex:InvalidInstanceR a ex:SingleLineExampleShape ; rdfs:label \"Invalid\\u000Dinstance\" .
ex:InvalidInstanceV a ex:SingleLineExampleShape ; rdfs:label \"Invalid\\u000Binstance\" .
ex:SingleLineExampleShape a sh:NodeShape, rdfs:Class ;
  sh:property ex:SingleLineExampleShape-label ; sh:property ex:SingleLineExampleShape-comment .
ex:SingleLineExampleShape-label a sh:PropertyShape ; sh:path rdfs:label ;
  sh:datatype xsd:string ; sh:singleLine true .
ex:SingleLineExampleShape-comment a sh:PropertyShape ; sh:path rdfs:comment ;
  sh:or ( [ sh:datatype xsd:string ] [ sh:datatype rdf:langString ] ) ; sh:singleLine false .
ex:ValidInstance1 a ex:SingleLineExampleShape ; rdfs:label \"Valid instance1\" ;
  rdfs:comment \"\"\"This is a multi-\nline comment.\n\"\"\" .
";
    let r = run(doc);
    let label_path = Some(Path::Predicate(
        "http://www.w3.org/2000/01/rdf-schema#label".into(),
    ));
    let mk = |focus: &str, value: Term| Exp {
        focus: ex(focus),
        path: label_path.clone(),
        value: Some(value),
        component: sh_comp("SingleLineConstraintComponent"),
        severity: format!("{SH}Violation"),
    };
    assert_report(
        &r,
        false,
        &[
            mk("InvalidInstanceF", lit("Invalid\u{000C}instance")),
            mk("InvalidInstanceN", lit("Invalid\u{000A}instance")),
            mk("InvalidInstanceR", lit("Invalid\u{000D}instance")),
            mk("InvalidInstanceV", lit("Invalid\u{000B}instance")),
        ],
    );
}

/// `rootClass-001`: `sh:rootClass ex:Animal` — a value must be ex:Animal or a
/// transitive `rdfs:subClassOf`-descendant. ex:Plant / ex:Car fail; ex:Animal /
/// ex:Mammal / ex:Dog (subclasses) pass.
#[test]
fn root_class_001() {
    let doc = r#"
ex:Organism a rdfs:Class .
ex:Animal a rdfs:Class ; rdfs:subClassOf ex:Organism .
ex:Mammal a rdfs:Class ; rdfs:subClassOf ex:Animal .
ex:Dog a rdfs:Class ; rdfs:subClassOf ex:Mammal .
ex:Plant a rdfs:Class ; rdfs:subClassOf ex:Organism .
ex:Car a rdfs:Class .
ex:Zoo ex:holds ex:Animal, ex:Mammal, ex:Dog, ex:Plant, ex:Car .
ex:TestShape a sh:NodeShape ; sh:targetNode ex:Zoo ; sh:property ex:TestShape-holds .
ex:TestShape-holds a sh:PropertyShape ; sh:path ex:holds ; sh:rootClass ex:Animal .
"#;
    let r = run(doc);
    let p = Some(pred_path("holds"));
    assert_report(
        &r,
        false,
        &[
            violation(ex("Zoo"), p.clone(), Some(ex("Plant")), "RootClassConstraintComponent"),
            violation(ex("Zoo"), p, Some(ex("Car")), "RootClassConstraintComponent"),
        ],
    );
}

// ============================ D. severity-threshold conforms ===============

/// `severity-004`: a `sh:Debug`-severity result is reported but does NOT break
/// conformance (Debug is below the default `sh:Info` threshold).
#[test]
fn severity_004_debug_conforms() {
    let doc = r#"
ex:TestShape a sh:NodeShape ; sh:datatype xsd:integer ; sh:severity sh:Debug ;
  sh:targetNode "Hello" .
"#;
    let r = run(doc);
    // One Debug-severity DatatypeConstraintComponent result, yet conforms = true.
    assert!(r.conforms, "Debug-only report must conform: {}", summarize(&r.results));
    assert_eq!(r.results.len(), 1);
    assert_eq!(r.results[0].severity, format!("{SH}Debug"));
    assert!(r.results[0].source_component.ends_with("DatatypeConstraintComponent"));
}

/// `severity-005`: a `sh:Trace`-severity result conforms (Trace is the lowest).
#[test]
fn severity_005_trace_conforms() {
    let doc = r#"
ex:TestShape a sh:NodeShape ; sh:datatype xsd:integer ; sh:severity sh:Trace ;
  sh:targetNode "Hello" .
"#;
    let r = run(doc);
    assert!(r.conforms, "Trace-only report must conform: {}", summarize(&r.results));
    assert_eq!(r.results.len(), 1);
    assert_eq!(r.results[0].severity, format!("{SH}Trace"));
}

/// A `sh:Warning`-severity result DOES break the DEFAULT conformance (Warning is
/// in the default disallowed set) — guards against over-relaxing `conforms`.
/// This mirrors the suite's `severity-001` / `conformance-disallows-002`.
#[test]
fn warning_breaks_default_conformance() {
    let doc = r#"
ex:PersonShape a sh:NodeShape ; sh:property ex:PersonShape-age ; sh:targetClass ex:Person .
ex:PersonShape-age a sh:PropertyShape ; sh:path ex:age ; sh:severity sh:Warning ;
  sh:datatype xsd:integer .
ex:Bob a ex:Person ; ex:age "twenty two" .
"#;
    let r = run(doc);
    assert!(!r.conforms, "a Warning result must break default conformance");
    assert_eq!(r.results.len(), 1);
    assert_eq!(r.results[0].severity, format!("{SH}Warning"));
}

/// `conformance-disallows-001`: the same Warning result CONFORMS under a custom
/// `sh:conformanceDisallows sh:Violation` (only Violation disallowed), recomputed
/// via `conforms_with_disallowed`. The default `conforms` field stays false.
#[test]
fn conformance_disallows_001_custom_threshold() {
    let doc = r#"
ex:PersonShape a sh:NodeShape ; sh:property ex:PersonShape-age ; sh:targetClass ex:Person .
ex:PersonShape-age a sh:PropertyShape ; sh:path ex:age ; sh:severity sh:Warning ;
  sh:datatype xsd:integer .
ex:Bob a ex:Person ; ex:age "twenty two" .
"#;
    let r = run(doc);
    // The default disallowed set includes sh:Warning -> field is false.
    assert!(!r.conforms);
    // Under the custom set {sh:Violation}, the Warning result conforms.
    assert!(r.conforms_with_disallowed(&["http://www.w3.org/ns/shacl#Violation"]));
    // The single result is the expected Warning datatype violation on ex:Bob.
    assert_eq!(r.results.len(), 1);
    let res = &r.results[0];
    assert_eq!(res.focus_node, ex("Bob"));
    assert_eq!(res.path, Some(pred_path("age")));
    assert_eq!(res.value, Some(lit("twenty two")));
    assert_eq!(res.severity, format!("{SH}Warning"));
    assert!(res.source_component.ends_with("DatatypeConstraintComponent"));
}
