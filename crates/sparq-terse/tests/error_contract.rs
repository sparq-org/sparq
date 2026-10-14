// sq-bif.33 — pin the public loud-failure and pass-through contracts.

use sparq_terse::{terse_to_sparql, TerseError, LEGEND_VERSION};

#[test]
fn unknown_keyword_exposes_versioned_suggestions() {
    let query = "SELECT ?f WHERE { ?f K:derived ?s }";
    let error = terse_to_sparql(query).expect_err("an unknown keyword must fail loudly");

    match error {
        TerseError::UnknownKeyword {
            keyword,
            legend_version,
            suggestions,
        } => {
            assert_eq!(keyword, "derived");
            assert_eq!(legend_version, LEGEND_VERSION);
            assert!(
                suggestions.contains(&"derivedFrom".to_string()),
                "expected derivedFrom among {suggestions:?}"
            );
        }
        other => panic!("expected UnknownKeyword, got {other:?}"),
    }
}

#[test]
fn real_prefix_k_collides_with_keyword_sigil() {
    let query = "PREFIX K: <http://ex/> SELECT ?s WHERE { ?s K:type ?o }";
    let error = terse_to_sparql(query).expect_err("PREFIX K: must collide with the K: sigil");

    match error {
        TerseError::KeywordPrefixCollision { keyword } => assert_eq!(keyword, "type"),
        other => panic!("expected KeywordPrefixCollision, got {other:?}"),
    }
}

#[test]
fn keyword_text_inside_literal_is_data() {
    let query = "SELECT ?s WHERE { ?s <http://ex/p> \"K:label\" }";
    let expansion = terse_to_sparql(query).expect("K: text inside a literal is data");

    assert_eq!(expansion.canonical_sparql, query);
    assert!(expansion.keywords.is_empty());
}

#[test]
fn vector_phrase_requires_opt_in_feature() {
    let query = "SELECT ?f WHERE { ?f <http://ex/about> V(\"cardinality estimation\") }";
    let error = terse_to_sparql(query).expect_err("V() requires the opt-in vectors surface");

    match error {
        TerseError::FeatureRequired { phrase, .. } => {
            assert_eq!(phrase, "cardinality estimation");
        }
        other => panic!("expected FeatureRequired, got {other:?}"),
    }
}

#[test]
fn canonical_sparql_passes_through_byte_identically() {
    let query = "SELECT ?s WHERE { ?s <http://ex/p> ?o }";
    let expansion = terse_to_sparql(query).expect("canonical SPARQL must pass through");

    assert_eq!(expansion.canonical_sparql, query);
    assert!(expansion.resolutions.is_empty());
    assert!(expansion.warnings.is_empty());
}
