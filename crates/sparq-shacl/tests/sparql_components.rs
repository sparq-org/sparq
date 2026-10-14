//! SHACL §6 — SPARQL-based constraint COMPONENTS (sq-sm2).
//!
//! Local-fixture tests for custom `sh:ConstraintComponent` declarations: a
//! component declares `sh:parameter`s and a validator (`sh:validator` /
//! `sh:nodeValidator` / `sh:propertyValidator`) carrying `sh:ask` or `sh:select`.
//! It activates on a shape that uses its parameter predicates; the parameter
//! values are pre-bound as `$paramName`, along with `$this` / `$value`, and the
//! validator runs.
//!
//! These use LOCAL fixtures (the component is declared in the shapes graph) and
//! complement the W3C `sparql/component/*` suite runner (`w3c_sparql_component.rs`,
//! sq-wys), which resolves that suite's `owl:imports <http://datashapes.org/dash>`
//! against a vendored excerpt. The component machinery is the same shape the dash
//! suite exercises (modelled on `tests/shacl/.../sparql/component/validator-001.ttl`).

use sparq_core::Graph;
use sparq_shacl::validate;

fn run(data: &str, shapes: &str) -> sparq_shacl::ValidationReport {
    let data = Graph::load_str(data, "turtle").unwrap();
    let shapes = Graph::load_str(shapes, "turtle").unwrap();
    validate(&data, &shapes)
}

const PREFIXES: &str = r#"
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix ex: <http://example.org/> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

/// A single-parameter ASK-validator component: `ex:maxLen` requires each value
/// node's string form to be AT MOST `$maxLen` characters. The validator runs per
/// value node; ASK=false (too long) → one violation, with the component's own
/// `sh:value` (the value node) and `sh:message`.
///
/// Mirrors the W3C `validator-001` structure (a component declared via a
/// subclass of `sh:ConstraintComponent`, an ASK validator referencing `$value`
/// and a `$param`), but the data violates/conforms LOCALLY. The ASK is a
/// FILTER-only pattern — exercising the pre-binding push-down (the pre-bound
/// `$value` / `$maxLen` must be in scope INSIDE the FILTER).
const MAX_LEN_COMPONENT: &str = r#"
    # The component is typed via a SUBCLASS of sh:ConstraintComponent (as the
    # W3C suite does) — discovery must follow the rdfs:subClassOf closure.
    ex:MyConstraintComponent rdfs:subClassOf sh:ConstraintComponent .
    ex:MaxLenComponent a ex:MyConstraintComponent ;
      sh:parameter [ sh:path ex:maxLen ] ;
      sh:validator [
        a sh:SPARQLAskValidator ;
        sh:message "Value is longer than {$maxLen} characters" ;
        sh:ask "ASK { FILTER (STRLEN(STR($value)) <= $maxLen) }" ;
      ] .
"#;

#[test]
fn ask_component_flags_and_conforms() {
    let shapes = format!(
        r#"{PREFIXES}
        {MAX_LEN_COMPONENT}
        ex:S a sh:NodeShape ;
          sh:targetNode "abcdef", "ab" ;
          # Using the component's parameter predicate activates it on this shape.
          ex:maxLen 3 .
    "#
    );
    // "abcdef" (6) > 3 -> violation; "ab" (2) <= 3 -> conforms.
    let r = run("@prefix ex: <http://example.org/> . ex:x ex:y ex:z .", &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    let res = &r.results[0];
    // sh:value is the value node ($value = the focus, for a node shape).
    assert_eq!(res.value.as_ref().unwrap().to_string(), "\"abcdef\"");
    // The component's own IRI is the sh:sourceConstraintComponent.
    assert_eq!(res.source_component, "http://example.org/MaxLenComponent");
    // The validator's sh:message with {$param} substitution ($maxLen is a
    // pre-bound VALUE, so {$maxLen} resolves to 3).
    let msg = res.effective_messages()[0].to_string();
    assert!(msg.contains("longer than 3 characters"), "message: {msg}");
}

#[test]
fn ask_component_does_not_activate_without_parameter() {
    // A shape that does NOT use ex:maxLen must not trigger the component, even
    // though the data would otherwise violate it.
    let shapes = format!(
        r#"{PREFIXES}
        {MAX_LEN_COMPONENT}
        ex:S a sh:NodeShape ;
          sh:targetNode "abcdef" ;
          sh:property [ sh:path ex:name ; sh:minCount 0 ] .
    "#
    );
    let r = run("@prefix ex: <http://example.org/> . ex:x ex:y ex:z .", &shapes);
    assert!(r.conforms, "component must not fire when its parameter is unused: {}", r.to_text());
}

/// Multi-parameter binding: a two-parameter ASK component (the W3C
/// `validator-001` "concat" pattern) flags any value that is NOT the
/// concatenation of `$prefix` and `$suffix`. Exercises pre-binding two distinct
/// `$paramName` variables in one VALUES row.
#[test]
fn multi_parameter_component() {
    let shapes = format!(
        r#"{PREFIXES}
        ex:ConcatComponent a sh:ConstraintComponent ;
          sh:parameter [ sh:path ex:prefix ] ;
          sh:parameter [ sh:path ex:suffix ] ;
          sh:validator [
            a sh:SPARQLAskValidator ;
            sh:ask "ASK {{ FILTER (str($value) = CONCAT(str($prefix), str($suffix))) }}" ;
          ] .
        ex:S a sh:NodeShape ;
          sh:targetNode "Hello World", "Goodbye" ;
          ex:prefix "Hello " ;
          ex:suffix "World" .
    "#
    );
    // The targets are literals; "Hello World" = "Hello "+"World" conforms,
    // "Goodbye" does not.
    let r = run("@prefix ex: <http://example.org/> . ex:x ex:y ex:z .", &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    let v = r.results[0].value.as_ref().unwrap().to_string();
    assert!(v.contains("Goodbye"), "the non-concat value should violate; got {v}");
    assert_eq!(r.results[0].source_component, "http://example.org/ConcatComponent");
}

/// A SELECT validator: each returned solution is one violation, with the §5.2
/// `?value` / `sh:message {?var}` mapping. The component flags every `ex:item`
/// of the focus that exceeds `$limit`.
#[test]
fn select_validator_component() {
    let shapes = format!(
        r#"{PREFIXES}
        ex:OverLimitComponent a sh:ConstraintComponent ;
          sh:parameter [ sh:path ex:limit ] ;
          sh:validator [
            a sh:SPARQLSelectValidator ;
            sh:message "Item {{?value}} exceeds {{$limit}}" ;
            sh:select """
              SELECT $this ?value WHERE {{
                $this <http://example.org/item> ?value .
                FILTER (?value > $limit)
              }}
            """ ;
          ] .
        ex:S a sh:NodeShape ;
          sh:targetNode ex:cart ;
          ex:limit 10 .
    "#
    );
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:cart ex:item 5, 12, 20 .   # 12 and 20 exceed 10
    "#;
    let r = run(data, &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 2, "{}", r.to_text());
    for res in &r.results {
        assert_eq!(res.source_component, "http://example.org/OverLimitComponent");
        let v = res.value.as_ref().unwrap().to_string();
        assert!(v.contains("12") || v.contains("20"), "value {v}");
        let msg = res.effective_messages()[0].to_string();
        assert!(msg.contains("exceeds 10"), "message: {msg}");
    }
}

/// `sh:optional` parameter: a component with one mandatory and one optional
/// parameter activates when only the mandatory one is present (the optional
/// `$paramName` is simply not pre-bound).
#[test]
fn optional_parameter() {
    let shapes = format!(
        r#"{PREFIXES}
        ex:RangeComponent a sh:ConstraintComponent ;
          sh:parameter [ sh:path ex:minVal ] ;
          sh:parameter [ sh:path ex:maxVal ; sh:optional true ] ;
          sh:validator [
            a sh:SPARQLAskValidator ;
            # Uses only $minVal here; $maxVal may be absent.
            sh:ask "ASK {{ FILTER (?value >= $minVal) }}" ;
          ] .
        ex:S a sh:NodeShape ;
          sh:targetNode ex:a, ex:b ;
          sh:path ex:score ;
          ex:minVal 5 .
    "#
    );
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:a ex:score 3 .   # 3 < 5 -> violation
        ex:b ex:score 7 .   # 7 >= 5 -> conforms
    "#;
    let r = run(data, &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert!(r.results[0].focus_node.to_string().contains("/a"));
}

/// A `sh:nodeValidator` is preferred over a generic `sh:validator` for a node
/// shape (SHACL §6.2.2). Here only the node validator would fire; the generic
/// one is intentionally a never-violating ASK, so seeing the node-validator's
/// message confirms it was the one chosen.
#[test]
fn node_validator_preferred_for_node_shape() {
    let shapes = format!(
        r#"{PREFIXES}
        ex:KindComponent a sh:ConstraintComponent ;
          sh:parameter [ sh:path ex:mustBeIri ] ;
          sh:validator [ a sh:SPARQLAskValidator ; sh:ask "ASK {{}}" ] ;
          sh:nodeValidator [
            a sh:SPARQLAskValidator ;
            sh:message "node-validator fired" ;
            sh:ask "ASK {{ FILTER (isIRI($value)) }}" ;
          ] .
        ex:S a sh:NodeShape ;
          sh:targetNode "a literal focus" ;
          ex:mustBeIri true .
    "#
    );
    let r = run("@prefix ex: <http://example.org/> . ex:x ex:y ex:z .", &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    let msg = r.results[0].effective_messages()[0].to_string();
    assert!(msg.contains("node-validator fired"), "message: {msg}");
}

/// (sq-wys) A `sh:propertyValidator` with `$PATH` pre-bound to the
/// property shape's path. Mirrors the W3C `propertyValidator-select-001` shape:
/// a SELECT validator over `$this $PATH ?value` flags values that are not
/// literals tagged with the parameter language `$lang`. `$PATH` is a property
/// PATH, not a term, so it cannot ride the VALUES pre-binding table — the
/// validator is re-parsed per property shape with the path substituted.
#[test]
fn property_validator_path_prebinding() {
    let shapes = format!(
        r#"{PREFIXES}
        ex:LangComponent a sh:ConstraintComponent ;
          sh:parameter [ sh:path ex:lang ; sh:name "language" ] ;
          sh:propertyValidator [
            a sh:SPARQLSelectValidator ;
            sh:select """
              SELECT DISTINCT $this ?value WHERE {{
                $this $PATH ?value .
                FILTER (!isLiteral(?value) || !langMatches(lang(?value), $lang))
              }}
            """ ;
          ] .
        # A property shape (has sh:path) activates the property validator; the
        # shape's path ex:englishLabel is substituted for $PATH.
        ex:S a sh:NodeShape ;
          sh:targetNode ex:country ;
          sh:property [ sh:path ex:englishLabel ; ex:lang "en" ] .
    "#
    );
    let data = r#"
        @prefix ex: <http://example.org/> .
        ex:country ex:englishLabel "Munich" ;        # no language tag -> violation
                   ex:englishLabel "Beijing"@en .     # @en -> conforms
    "#;
    let r = run(data, &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    let res = &r.results[0];
    assert_eq!(res.value.as_ref().unwrap().to_string(), "\"Munich\"");
    // The result path is the property shape's path (inherited), not a $PATH leak.
    assert_eq!(
        res.path.as_ref().map(|p| p.to_turtle()),
        Some("<http://example.org/englishLabel>".to_string())
    );
    assert_eq!(res.source_component, "http://example.org/LangComponent");

    // The same component on a DIFFERENT-path shape re-binds $PATH to that path —
    // proving the substitution is per-shape, not shared.
    let shapes2 = format!(
        r#"{PREFIXES}
        ex:LangComponent a sh:ConstraintComponent ;
          sh:parameter [ sh:path ex:lang ; sh:name "language" ] ;
          sh:propertyValidator [
            a sh:SPARQLSelectValidator ;
            sh:select """
              SELECT DISTINCT $this ?value WHERE {{
                $this $PATH ?value .
                FILTER (!isLiteral(?value) || !langMatches(lang(?value), $lang))
              }}
            """ ;
          ] .
        ex:S a sh:NodeShape ;
          sh:targetNode ex:country ;
          sh:property [ sh:path ex:germanLabel ; ex:lang "de" ] .
    "#
    );
    let data2 = r#"
        @prefix ex: <http://example.org/> .
        ex:country ex:englishLabel "Munich" ;     # different predicate: ignored
                   ex:germanLabel "Muenchen" .    # no @de -> violation
    "#;
    let r2 = run(data2, &shapes2);
    assert_eq!(r2.results.len(), 1, "{}", r2.to_text());
    assert_eq!(r2.results[0].value.as_ref().unwrap().to_string(), "\"Muenchen\"");
}

/// (sq-wys) The pre-bound parameter variable is the LOCAL NAME of the
/// parameter's `sh:path` IRI, NOT its `sh:name` display label (SHACL §6.2.1).
/// Here the parameter is `sh:path ex:lang ; sh:name "language"` and the validator
/// references `$lang`; binding `$language` would leave `$lang` unbound and the
/// constraint would never fire.
#[test]
fn parameter_variable_is_path_local_name_not_sh_name() {
    let shapes = format!(
        r#"{PREFIXES}
        ex:LangComponent a sh:ConstraintComponent ;
          sh:parameter [ sh:path ex:lang ; sh:name "language" ] ;
          sh:validator [
            a sh:SPARQLAskValidator ;
            sh:ask "ASK {{ FILTER (isLiteral($value) && langMatches(lang($value), $lang)) }}" ;
          ] .
        ex:S a sh:NodeShape ;
          sh:targetNode "Munich", "Beijing"@en ;
          ex:lang "en" .
    "#
    );
    let r = run("@prefix ex: <http://example.org/> . ex:x ex:y ex:z .", &shapes);
    assert!(!r.conforms, "{}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    // "Munich" (no @en) violates; "Beijing"@en conforms.
    assert_eq!(r.results[0].value.as_ref().unwrap().to_string(), "\"Munich\"");
}

/// The report Turtle for a custom-component result is valid Turtle and carries
/// the component's IRI as sh:sourceConstraintComponent.
#[test]
fn component_report_turtle_parses() {
    let shapes = format!(
        r#"{PREFIXES}
        {MAX_LEN_COMPONENT}
        ex:S a sh:NodeShape ; sh:targetNode "abcdef" ; ex:maxLen 1 .
    "#
    );
    let r = run("@prefix ex: <http://example.org/> . ex:x ex:y ex:z .", &shapes);
    assert!(!r.conforms);
    let ttl = r.to_turtle();
    let parsed: Result<Vec<_>, _> = oxttl::TurtleParser::new().for_slice(ttl.as_bytes()).collect();
    let triples = parsed.unwrap_or_else(|e| panic!("report Turtle does not parse: {e}\n{ttl}"));
    assert!(triples
        .iter()
        .any(|t| t.object.to_string().contains("MaxLenComponent")));
}
