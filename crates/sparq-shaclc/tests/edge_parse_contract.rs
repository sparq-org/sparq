// Integration contracts for the crate-root SHACL-C parse API.
use sparq_shaclc::{parse_extended, parse_strict, DEFAULT_BASE};

#[test]
fn strict_parse_reports_predeclared_prefixes_and_default_base() {
    let (_, outcome) = parse_strict("shape <http://e/S> {\n}\n", DEFAULT_BASE)
        .expect("strict parser should accept an empty absolute-IRI shape");

    let prefix_names: Vec<&str> = outcome
        .prefixes
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(prefix_names, ["rdf", "rdfs", "sh", "xsd", "owl"]);
    assert_eq!(outcome.base, DEFAULT_BASE);
}

#[test]
fn undeclared_prefix_has_stable_code_and_display_position() {
    let error = parse_strict("shape nope:S {\n}\n", DEFAULT_BASE)
        .expect_err("strict parser should reject an undeclared prefix");

    assert_eq!(error.code, Some("UNDECLARED_PREFIX"));
    assert!(
        error.to_string().contains("line 1"),
        "position missing from {error}"
    );
}

#[test]
fn annotation_list_is_extended_only_and_strict_error_is_positioned() {
    let document = "PREFIX ex: <http://example.org/test#>\nshape ex:S ;\n  ex:myProperty 1\n{\n}\n";

    let strict_error = parse_strict(document, DEFAULT_BASE)
        .expect_err("strict parser should reject the annotation-list extension");
    assert!(
        strict_error.line >= 2,
        "extension error was not positioned: {strict_error}"
    );
    assert!(
        strict_error.code.is_some(),
        "stable error code missing: {strict_error:?}"
    );

    parse_extended(document, DEFAULT_BASE)
        .expect("extended parser should accept the annotation-list extension");
}
