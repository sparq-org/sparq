//! [OPUS-5.5] Stable-parser contract for `parse_versioned_query`/`_update`.
//!
//! Only APIs present in the published `spargebra` 0.4.6 appear here, so the
//! registry-only consumer gate (`scripts/ci_registry_parser.py`) compiles and
//! runs this same file against upstream, once per Unicode escaping mode. Set
//! `SPARQ_EXPECT_STANDARD_UNICODE_ESCAPING` to `0` or `1` to require a mode.
//! Keep it free of crate-feature `cfg`s: the gate compiles it inside another
//! crate, where those features would not mean the engine's.

#[path = "parser_versions/cases.rs"]
mod cases;

use cases::{Case, Expect};
use spargebra::algebra::GraphPattern;
use spargebra::term::NamedNode;
use spargebra::{Query, SparqlParser};
use sparq_engine::{
    parse_update_rec2013, parse_versioned_query, parse_versioned_update, EbvSemantics, PreparedQuery,
    VersionedParseError,
};

fn mode_name(preprocessing: bool) -> &'static str {
    if preprocessing {
        "standard-unicode-escaping"
    } else {
        "verbatim"
    }
}

/// Checks one adapter outcome against the corpus and the parser's own verdict.
fn assert_outcome(
    case: &Case,
    preprocessing: bool,
    outcome: Result<Vec<String>, VersionedParseError>,
    direct: Result<(), String>,
) {
    let mode = mode_name(preprocessing);
    match (case.expected(preprocessing), outcome) {
        (Expect::Labels(expected), Ok(versions)) => {
            assert_eq!(direct, Ok(()), "{} ({mode}): the parser must accept", case.name);
            assert_eq!(versions, expected, "{} ({mode})", case.name);
        }
        (Expect::Syntax, Err(error)) => {
            assert!(error.is_syntax() && !error.is_prologue(), "{} ({mode}): {error}", case.name);
            // The diagnostic is the parser's own text, unchanged.
            assert_eq!(Err(error.to_string()), direct, "{} ({mode})", case.name);
        }
        (expected, outcome) => {
            panic!("{} ({mode}): expected {expected:?}, got {outcome:?}", case.name)
        }
    }
}

#[test]
fn query_corpus_matches_the_active_escaping_mode() {
    let preprocessing = cases::linked_parser_preprocesses();
    for case in cases::QUERY_CASES {
        let direct = SparqlParser::new().parse_query(case.text).map(drop).map_err(|error| error.to_string());
        let outcome = parse_versioned_query(SparqlParser::new(), case.text).map(|(_, versions)| versions);
        assert_outcome(case, preprocessing, outcome, direct);
    }
}

#[test]
fn update_corpus_matches_the_active_escaping_mode() {
    let preprocessing = cases::linked_parser_preprocesses();
    for case in cases::UPDATE_CASES {
        let direct = SparqlParser::new().parse_update(case.text).map(drop).map_err(|error| error.to_string());
        let outcome = parse_versioned_update(SparqlParser::new(), case.text).map(|(_, versions)| versions);
        assert_outcome(case, preprocessing, outcome, direct);
    }
}

#[test]
fn requested_unicode_escaping_mode_is_the_linked_mode() {
    // The gate sets this per run, so a feature that silently failed to apply
    // (or was unified on from elsewhere) cannot pass as the other mode.
    let Ok(requested) = std::env::var("SPARQ_EXPECT_STANDARD_UNICODE_ESCAPING") else {
        return;
    };
    let requested = match requested.as_str() {
        "0" => false,
        "1" => true,
        other => panic!("SPARQ_EXPECT_STANDARD_UNICODE_ESCAPING must be 0 or 1, not {other:?}"),
    };
    assert_eq!(
        cases::linked_parser_preprocesses(),
        requested,
        "linked spargebra escaping mode differs from the requested build"
    );
}

/// Whether a query's algebra contains a GROUP, i.e. parsed an aggregate.
fn contains_group(pattern: &GraphPattern) -> bool {
    match pattern {
        GraphPattern::Group { .. } => true,
        GraphPattern::Project { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::Filter { inner, .. }
        | GraphPattern::Distinct { inner, .. }
        | GraphPattern::Reduced { inner, .. }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::OrderBy { inner, .. } => contains_group(inner),
        _ => false,
    }
}

fn select_pattern(query: &Query) -> &GraphPattern {
    match query {
        Query::Select { pattern, .. } => pattern,
        other => panic!("expected SELECT, got {other}"),
    }
}

#[test]
fn configured_base_prefix_and_aggregate_options_are_preserved() {
    let text = "BASE <rel/> VERSION '1.2' SELECT * WHERE { <s> <p> ?o }";
    let based = SparqlParser::new().with_base_iri("http://ex/").unwrap();
    let (query, versions) = parse_versioned_query(based, text).unwrap();
    assert_eq!(versions, ["1.2"]);
    assert_eq!(query.base_iri().map(|iri| iri.as_str()), Some("http://ex/rel/"));
    assert!(parse_versioned_query(SparqlParser::new(), text).unwrap_err().is_syntax());

    let text = "VERSION \"1.1\" ASK { ex:s ex:p ex:o }";
    let prefixed = SparqlParser::new().with_prefix("ex", "http://ex/").unwrap();
    assert_eq!(parse_versioned_query(prefixed, text).unwrap().1, ["1.1"]);
    assert!(parse_versioned_query(SparqlParser::new(), text).unwrap_err().is_syntax());

    let text = "VERSION '1.2' SELECT (<http://ex/agg>(?x) AS ?y) WHERE { VALUES ?x { 1 } }";
    let aggregating =
        SparqlParser::new().with_custom_aggregate_function(NamedNode::new("http://ex/agg").unwrap());
    let (declared, versions) = parse_versioned_query(aggregating, text).unwrap();
    assert_eq!(versions, ["1.2"]);
    assert!(contains_group(select_pattern(&declared)), "declared aggregate must group");
    let (plain, _) = parse_versioned_query(SparqlParser::new(), text).unwrap();
    assert!(!contains_group(select_pattern(&plain)), "undeclared IRI is a scalar call");
}

#[test]
fn syntax_errors_keep_the_parser_diagnostic_at_every_entry_point() {
    for text in ["SELECT", "VERSION '1.2", "ASK {} VERSION '1.2'", "VERSION '1\\q' ASK {}"] {
        let error = parse_versioned_query(SparqlParser::new(), text).unwrap_err();
        let direct = SparqlParser::new().parse_query(text).unwrap_err().to_string();
        assert!(error.is_syntax() && !error.is_prologue(), "{text:?}");
        assert_eq!(error.to_string(), direct, "{text:?}");
        assert_eq!(PreparedQuery::parse(text).unwrap_err(), direct, "{text:?}");
    }
    let text = "VERSION '1.1' INSERT DATA {} VERSION '1.1'";
    let error = parse_versioned_update(SparqlParser::new(), text).unwrap_err();
    let direct = SparqlParser::new().parse_update(text).unwrap_err().to_string();
    assert!(error.is_syntax(), "{text:?}");
    assert_eq!(error.to_string(), direct);
    assert_eq!(parse_update_rec2013(text).unwrap_err(), direct);
}

#[test]
fn label_checks_stay_after_syntax_and_before_evaluation() {
    // Syntax acceptance retains every label; preparation alone judges them.
    let (query, versions) = parse_versioned_query(SparqlParser::new(), "VERSION 'bogus' ASK {}").unwrap();
    assert_eq!(versions, ["bogus"]);
    assert_eq!(
        PreparedQuery::from_query_with_versions(query, versions).unwrap_err(),
        "unsupported SPARQL VERSION announcement"
    );
    assert_eq!(
        PreparedQuery::parse("VERSION '1.1' VERSION '1.2' ASK {}").unwrap_err(),
        "SPARQL VERSION announcements require incompatible EBV semantics"
    );
    let prepared = PreparedQuery::parse("# VERSION '1.1'\nVERSION '1.2' VERSION '1.2-basic' ASK {}").unwrap();
    assert_eq!(prepared.versions(), ["1.2", "1.2-basic"]);
    assert_eq!(prepared.resolve_ebv_semantics(None), Ok(EbvSemantics::Draft20260912));
    assert_eq!(
        prepared.resolve_ebv_semantics(Some(EbvSemantics::Rec2013)).unwrap_err(),
        "SPARQL VERSION contradicts explicit EBV semantics"
    );
    // UPDATE keeps its REC 2013-only refusal, including a prologue-only update.
    assert!(parse_update_rec2013("VERSION '1.1' INSERT DATA {}").is_ok());
    assert!(parse_update_rec2013("VERSION '1.1'").is_ok());
    for text in ["VERSION '1.2' INSERT DATA {}", "VERSION '1.1' VERSION 'bogus'"] {
        assert_eq!(
            parse_update_rec2013(text).unwrap_err(),
            "UPDATE supports only REC 2013 EBV semantics (VERSION 1.1)",
            "{text:?}"
        );
    }
}
