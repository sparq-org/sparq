//! (sq-lsp7k.2.1) Feature-state and behavioural coverage for the
//! SHACL-AF validation fact-domain switch.

use sparq_core::Graph;

fn fixture() -> (Graph, Graph) {
    let data = Graph::load_str(
        r#"
        @prefix ex: <http://example.org/> .
        ex:a a ex:Person .
        "#,
        "turtle",
    )
    .unwrap();
    let shapes = Graph::load_str(
        r#"
        @prefix sh:  <http://www.w3.org/ns/shacl#> .
        @prefix ex:  <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:PersonShape a sh:NodeShape ;
          sh:targetClass ex:Person ;
          sh:rule [
            a sh:TripleRule ;
            sh:subject sh:this ;
            sh:predicate ex:age ;
            sh:object "x" ;
          ] ;
          sh:property [
            sh:path ex:age ;
            sh:datatype xsd:integer ;
          ] .
        "#,
        "turtle",
    )
    .unwrap();
    (data, shapes)
}

#[cfg(not(feature = "shacl-af"))]
#[test]
fn feature_off_keeps_base_validation_asserted_only() {
    let (data, shapes) = fixture();
    let report = sparq_shacl::validate(&data, &shapes);
    assert!(report.conforms, "SHACL-AF rule must remain compiled out");
    assert!(report.results.is_empty());
}

#[cfg(feature = "shacl-af")]
fn assert_inferred_violation(report: &sparq_shacl::ValidationReport) {
    assert!(!report.conforms);
    assert_eq!(report.results.len(), 1, "{}", report.to_text());
    let result = &report.results[0];
    assert_eq!(result.focus_node.to_string(), "<http://example.org/a>");
    assert_eq!(result.value.as_ref().unwrap().to_string(), "\"x\"");
    assert!(result
        .source_component
        .ends_with("DatatypeConstraintComponent"));
}

#[cfg(feature = "shacl-af")]
#[test]
fn validate_with_domain_switches_between_asserted_and_closure() {
    use sparq_shacl::{validate_with_domain, FactDomain};

    let (data, shapes) = fixture();
    let asserted = validate_with_domain(&data, &shapes, FactDomain::Asserted);
    assert!(asserted.conforms, "{}", asserted.to_text());
    assert!(asserted.results.is_empty());

    let expanded = validate_with_domain(&data, &shapes, FactDomain::AssertedPlusInferred);
    assert_inferred_violation(&expanded);
}

#[cfg(feature = "shacl-af")]
#[test]
fn validate_with_domain_and_model_reuses_model_for_both_domains() {
    use sparq_shacl::{validate_with_domain_and_model, FactDomain, ShapesModel};

    let (data, shapes) = fixture();
    let model = ShapesModel::parse(&shapes);
    let asserted = validate_with_domain_and_model(&data, &shapes, &model, FactDomain::Asserted);
    assert!(asserted.conforms, "{}", asserted.to_text());

    let expanded =
        validate_with_domain_and_model(&data, &shapes, &model, FactDomain::AssertedPlusInferred);
    assert_inferred_violation(&expanded);
}
