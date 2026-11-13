use sparq_jsonld::from_rdf::RdfTerm;
use sparq_jsonld::to_rdf::to_rdf;
use sparq_jsonld::{Json, JsonLdOptions, NoopLoader, RdfDirection};

fn convert(text: &str) -> Vec<sparq_jsonld::from_rdf::RdfQuad> {
    to_rdf(
        &Json::parse(text).unwrap(),
        &JsonLdOptions::default(),
        &NoopLoader,
    )
    .unwrap()
}

#[test]
fn list_preserves_cell_for_invalid_resource() {
    let quads = convert(
        r#"{"@id":"https://ex/s","https://ex/p":{"@list":[{"@id":"relative"},{"@value":"v"}]}}"#,
    );
    assert_eq!(quads.len(), 4);
    let rest: Vec<_> = quads
        .iter()
        .filter(|q| q.predicate == RdfTerm::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#rest"))
        .collect();
    assert_eq!(rest.len(), 2);
    assert_eq!(
        quads
            .iter()
            .filter(
                |q| q.predicate == RdfTerm::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#first")
            )
            .count(),
        1
    );
}

#[test]
fn invalid_resources_datatypes_language_and_null_ids_are_omitted() {
    assert!(convert(r#"{"@id":"https://ex/s","https://ex/p":[{"@id":"@Ignore"},{"@value":"v","@language":"bad tag"}] }"#).is_empty());
    assert!(convert(r#"{"@id":"https://bad iri/s","https://ex/p":"v"}"#).is_empty());
}

#[test]
fn numbers_and_json_literals_use_canonical_forms() {
    let quads = convert(
        r#"{"@id":"https://ex/s","https://ex/p":[-0e0,9.9,1e21,{"@type":"@json","@value":{"z":[3,2,1],"a":1.0}}]}"#,
    );
    let objects: Vec<_> = quads.into_iter().map(|q| q.object).collect();
    assert!(objects.contains(&RdfTerm::typed_literal(
        "0",
        "http://www.w3.org/2001/XMLSchema#integer"
    )));
    assert!(objects.contains(&RdfTerm::typed_literal(
        "9.9E0",
        "http://www.w3.org/2001/XMLSchema#double"
    )));
    assert!(objects.contains(&RdfTerm::typed_literal(
        "1.0E21",
        "http://www.w3.org/2001/XMLSchema#double"
    )));
    assert!(objects.contains(&RdfTerm::typed_literal(
        r#"{"a":1,"z":[3,2,1]}"#,
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#JSON"
    )));
}

#[test]
fn direction_encodings_and_generalized_predicates_are_opt_in_options() {
    let input = Json::parse(r#"{"@id":"https://ex/s","https://ex/p":{"@value":"v","@language":"EN","@direction":"rtl"}}"#).unwrap();
    let mut options = JsonLdOptions::default();
    options.rdf_direction = RdfDirection::I18nDatatype;
    let quads = to_rdf(&input, &options, &NoopLoader).unwrap();
    assert_eq!(
        quads[0].object,
        RdfTerm::typed_literal("v", "https://www.w3.org/ns/i18n#en_rtl")
    );
    options.rdf_direction = RdfDirection::CompoundLiteral;
    let quads = to_rdf(&input, &options, &NoopLoader).unwrap();
    assert_eq!(quads.len(), 4);
    let input = Json::parse(r#"{"@id":"https://ex/s","_:predicate":"v"}"#).unwrap();
    assert!(to_rdf(&input, &options, &NoopLoader).unwrap().is_empty());
    options.produce_generalized_rdf = true;
    let quads = to_rdf(&input, &options, &NoopLoader).unwrap();
    assert_eq!(quads.len(), 1);
    assert!(matches!(quads[0].predicate, RdfTerm::BlankNode(_)));
}

#[test]
fn numeric_overflow_returns_an_error() {
    let input = Json::parse(r#"{"https://ex/p":1e999}"#).unwrap();
    let error = to_rdf(&input, &JsonLdOptions::default(), &NoopLoader).unwrap_err();
    assert_eq!(
        error.code(),
        sparq_jsonld::JsonLdErrorCode::InvalidValueObjectValue
    );
}
