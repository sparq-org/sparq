//! [OPUS-5.5] Differential: the stable-parser adapter versus the vendored fork.
//!
//! The vendored `spargebra` fork retains VERSION labels itself. This target is
//! the only permitted caller of those fork-only methods
//! (`scripts/check_registry_parser.py scan` enforces it); production code uses
//! `sparq_engine::parse_versioned_{query,update}`, which also compiles against
//! upstream `spargebra` 0.4.6. The gate runs this target in both upstream
//! escaping modes: plainly and with
//! `--features spargebra/standard-unicode-escaping`.

#[path = "parser_versions/cases.rs"]
mod cases;

use cases::{Case, Expect};
use proptest::prelude::*;
use spargebra::SparqlParser;

/// Why the adapter and the fork disagree on a query, if they do.
fn query_disagreement(text: &str) -> Option<String> {
    let adapter = sparq_engine::parse_versioned_query(SparqlParser::new(), text);
    let fork = SparqlParser::new().parse_query_with_versions(text);
    match (adapter, fork) {
        (Ok(adapter), Ok(fork)) if adapter == fork => None,
        (Err(adapter), Err(fork)) if adapter.is_syntax() && adapter.to_string() == fork.to_string() => None,
        (adapter, fork) => Some(format!("{text:?}: adapter {adapter:?}, fork {fork:?}")),
    }
}

/// Why the adapter and the fork disagree on an update, if they do.
fn update_disagreement(text: &str) -> Option<String> {
    let adapter = sparq_engine::parse_versioned_update(SparqlParser::new(), text);
    let fork = SparqlParser::new().parse_update_with_versions(text);
    match (adapter, fork) {
        (Ok(adapter), Ok(fork)) if adapter == fork => None,
        (Err(adapter), Err(fork)) if adapter.is_syntax() && adapter.to_string() == fork.to_string() => None,
        (adapter, fork) => Some(format!("{text:?}: adapter {adapter:?}, fork {fork:?}")),
    }
}

/// The corpus expectation must also describe what the fork itself retains.
fn assert_fork_expectation(case: &Case, preprocessing: bool, fork: Option<Vec<String>>) {
    match (case.expected(preprocessing), fork) {
        (Expect::Labels(expected), Some(labels)) => assert_eq!(labels, expected, "{}", case.name),
        (Expect::Syntax, None) => {}
        (expected, fork) => panic!("{}: fork retained {fork:?}, corpus expects {expected:?}", case.name),
    }
}

#[test]
fn corpus_agrees_with_the_fork_in_the_active_escaping_mode() {
    let preprocessing = cases::linked_parser_preprocesses();
    for case in cases::QUERY_CASES {
        assert_eq!(query_disagreement(case.text), None, "{}", case.name);
        let fork = SparqlParser::new().parse_query_with_versions(case.text).ok();
        assert_fork_expectation(case, preprocessing, fork.map(|(_, labels)| labels));
    }
    for case in cases::UPDATE_CASES {
        assert_eq!(update_disagreement(case.text), None, "{}", case.name);
        let fork = SparqlParser::new().parse_update_with_versions(case.text).ok();
        assert_fork_expectation(case, preprocessing, fork.map(|(_, labels)| labels));
    }
}

/// A keyword with each ASCII letter independently upper- or lower-cased.
fn cased(keyword: &'static str) -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<bool>(), keyword.len()).prop_map(move |upper| {
        keyword
            .chars()
            .zip(upper)
            .map(|(c, upper)| if upper { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() })
            .collect()
    })
}

/// The parser's `_` rule, plus escaped separators that differ between modes.
fn gap() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        4 => prop_oneof![Just(""), Just(" "), Just("\t"), Just("\r\n"), Just("\n")],
        2 => prop_oneof![Just("# VERSION '1.1'\n"), Just("#VERSION\"x\"\r")],
        1 => prop_oneof![
            Just("#\\u000A VERSION '1.1'\n"),
            Just("# x\\U0000000A"),
            Just("\\u0020"),
            Just("\\u000A"),
        ],
    ]
}

/// Label fragments: plain text, every escape form, and characters that the
/// grammar rejects or that preprocessing turns into quotes or line breaks.
fn label_part() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        6 => prop_oneof![Just("1"), Just("."), Just("2"), Just("-basic"), Just("\u{e9}"), Just(" ")],
        3 => prop_oneof![
            Just("\\t"),
            Just("\\'"),
            Just("\\\""),
            Just("\\\\"),
            Just("\\u0041"),
            Just("\\U0001F600"),
        ],
        2 => prop_oneof![
            Just("\\u0022"),
            Just("\\u0027"),
            Just("\\u000A"),
            Just("\\uD800"),
            Just("\\q"),
            Just("\\"),
            Just("'"),
            Just("\""),
            Just("\n"),
            Just("\\\\u0031"),
            Just("\\u+031"),
        ],
    ]
}

fn label() -> impl Strategy<Value = String> {
    (prop_oneof![Just('\''), Just('"')], proptest::collection::vec(label_part(), 0..5))
        .prop_map(|(quote, parts)| format!("{quote}{}{quote}", parts.concat()))
}

fn declaration() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => (cased("VERSION"), gap(), label())
            .prop_map(|(keyword, separator, text)| format!("{keyword}{separator}{text}")),
        1 => (
            cased("BASE"),
            gap(),
            prop_oneof![Just("<http://ex/>"), Just("<rel/>"), Just("<http://ex/VERSION>"), Just("<http://ex/\\u0041/>")],
        )
            .prop_map(|(keyword, separator, iri)| format!("{keyword}{separator}{iri}")),
        1 => (
            cased("PREFIX"),
            gap(),
            prop_oneof![Just("ex:"), Just(":"), Just("a.b-c:"), Just("a.:"), Just("\\u0065x:"), Just("version:")],
            gap(),
        )
            .prop_map(|(keyword, before, name, after)| format!("{keyword}{before}{name}{after}<http://ex/p#>")),
        1 => prop_oneof![
            Just("VERSION".to_owned()),
            Just("\\u0056ERSION '1.2'".to_owned()),
            Just("\\u0023".to_owned()),
        ],
    ]
}

fn prologue() -> impl Strategy<Value = String> {
    (gap(), proptest::collection::vec((declaration(), gap()), 0..5)).prop_map(|(lead, declarations)| {
        let mut text = lead.to_owned();
        for (declared, separator) in declarations {
            text.push_str(&declared);
            text.push_str(separator);
        }
        text
    })
}

/// Deterministic bodies: no blank nodes or aggregates, so algebra compares equal.
fn query_body() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("ASK {}"),
        Just("SELECT * WHERE { ?s ?p \"VERSION '1.1'\" }"),
        Just("ASK { FILTER(\"\\u0041\" = \"A\") }"),
        Just("ASK { <http://ex/s> <http://ex/p> ?VERSION }"),
        Just("CONSTRUCT WHERE { ?s ?p ?o }"),
        Just(""),
        Just("VERSION '1.2' ASK {}"),
    ]
}

fn update_body() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just(""),
        Just("CLEAR ALL"),
        Just("INSERT DATA { <http://ex/s> <http://ex/p> \"VERSION '1.1'\" }"),
        Just("CLEAR ALL ; VERSION '1.2' CLEAR ALL"),
        Just("CLEAR ALL ;"),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        ..ProptestConfig::default()
    })]

    /// Generated prologues: acceptance, diagnostics, algebra and every label agree.
    #[test]
    fn generated_query_prologues_agree_with_the_fork((head, body) in (prologue(), query_body())) {
        prop_assert_eq!(query_disagreement(&format!("{head}{body}")), None);
    }

    /// The same for the update grammar, including prologue-only updates.
    #[test]
    fn generated_update_prologues_agree_with_the_fork((head, body) in (prologue(), update_body())) {
        prop_assert_eq!(update_disagreement(&format!("{head}{body}")), None);
    }
}
