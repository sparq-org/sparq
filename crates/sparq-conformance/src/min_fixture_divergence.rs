//! [GPT-6] Exact exception for the pinned grouped-MIN fixture transcription.
//!
//! The upstream fixture stays unchanged. Its query, input and expected bytes,
//! manifest identity, and complete single-cell mismatch must all match. The
//! diagnostic emits up to TWO rows on each side: requiring exactly ONE row
//! prevents a second changed value, omitted row or duplicate from being hidden.

use sparq_conformance::manifest::{EntryKind, TestEntry};
use std::path::Path;

const QUERY: &[u8] = include_bytes!("../tests/fixtures/agg-min-02/agg-min-02.rq");
const DATA: &[u8] = include_bytes!("../tests/fixtures/agg-min-02/agg-numeric.ttl");
const RESULT: &[u8] = include_bytes!("../tests/fixtures/agg-min-02/agg-min-02.srx");
const ID: &str =
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/aggregates/manifest#agg-min-02";
const DIAGNOSTIC: &str = concat!(
    "result mismatch: expected 5 solution(s), got 5; expected-only e.g. ",
    "{?min=\"2.0E-1\"^^<http://www.w3.org/2001/XMLSchema#double> ?s=<http://www.example.org/mixed2>}; ",
    "actual-only e.g. {?min=\"2E-1\"^^<http://www.w3.org/2001/XMLSchema#double> ?s=<http://www.example.org/mixed2>}"
);
const RATIONALE: &str = "agg-min-02.srx rewrites the input term 2E-1 as 2.0E-1 for :mixed2. \
    SPARQL 1.1 REC section 18.5.1.5 defines Min(S)=S0, selecting an original term; \
    the suite requires identical literal nodes. Only the exact pinned source and \
    single-cell mismatch are classified here; the strict comparator and upstream \
    expected file remain unchanged. See docs/upstream-proposals.md for provenance";

fn same_file(path: Option<&Path>, expected: &[u8]) -> bool {
    path.and_then(|p| std::fs::read(p).ok()).as_deref() == Some(expected)
}

pub(super) fn classify(entry: &TestEntry, reason: &str) -> Option<&'static str> {
    (reason == DIAGNOSTIC
        && entry.id == ID
        && entry.name == "MIN with GROUP BY"
        && entry.suite == "sparql11/aggregates"
        && entry.kind == EntryKind::QueryEval
        && !entry.withdrawn
        && entry.action.data.len() == 1
        && entry.action.graph_data.is_empty()
        && entry.action.unsupported_feature.is_none()
        && entry.action.entailment_regimes.is_empty()
        && entry.action.entailment_profiles.is_empty()
        && same_file(entry.action.query.as_deref(), QUERY)
        && same_file(entry.action.data.first().map(|p| p.as_path()), DATA)
        && same_file(entry.result_file.as_deref(), RESULT))
    .then_some(RATIONALE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_conformance::{
        manifest::{QueryAction, UpdateState},
        run::{Status, run_query_test},
    };
    use std::path::PathBuf;

    fn fixture() -> TestEntry {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agg-min-02");
        TestEntry {
            id: ID.into(),
            name: "MIN with GROUP BY".into(),
            suite: "sparql11/aggregates".into(),
            kind: EntryKind::QueryEval,
            withdrawn: false,
            action: QueryAction {
                query: Some(dir.join("agg-min-02.rq")),
                data: vec![dir.join("agg-numeric.ttl")],
                ..Default::default()
            },
            result_file: Some(dir.join("agg-min-02.srx")),
            update_request: None,
            update_pre: UpdateState::default(),
            update_post: UpdateState::default(),
        }
    }

    #[test]
    fn real_fixture_has_only_the_pinned_literal_mismatch() {
        let entry = fixture();
        let Status::Fail(reason) = run_query_test(&entry) else {
            panic!("upstream fixture or engine changed; retire/review this exception")
        };
        assert_eq!(reason, DIAGNOSTIC);
        assert!(classify(&entry, &reason).is_some());
    }

    #[test]
    fn altered_value_other_row_or_diagnostic_is_not_a_divergence() {
        let entry = fixture();
        for reason in [
            DIAGNOSTIC.replace("?min=\"2E-1\"", "?min=\"3E-1\""),
            format!(
                "{DIAGNOSTIC}, {{?min=\"2\"^^<http://www.w3.org/2001/XMLSchema#integer> ?s=<http://www.example.org/ints>}}"
            ),
            DIAGNOSTIC.replace("got 5", "got 6"),
            "engine error: arbitrary failure".into(),
        ] {
            assert!(classify(&entry, &reason).is_none(), "{reason}");
        }
    }

    #[test]
    fn real_changed_values_and_other_rows_remain_failures() {
        let entry = fixture();
        let graph =
            sparq_core::Graph::load_str(std::str::from_utf8(DATA).unwrap(), "turtle").unwrap();
        let triples: String = graph
            .iter_ids()
            .map(|row| {
                format!(
                    "{} {} {} .\n",
                    graph.dict.term(row[0]),
                    graph.dict.term(row[1]),
                    graph.dict.term(row[2])
                )
            })
            .collect();
        for (before, after) in [
            ("\"2E-1\"", "\"3E-1\""),
            (
                "<http://www.example.org/ints>",
                "<http://www.example.org/changed>",
            ),
        ] {
            let changed = triples.replace(before, after);
            assert_ne!(changed, triples);
            let Status::Fail(reason) =
                sparq_conformance::run::run_query_test_on(&entry, Some(changed))
            else {
                panic!("changed result must fail strict comparison")
            };
            assert!(classify(&entry, &reason).is_none(), "{reason}");
        }
    }

    #[test]
    fn changed_source_or_manifest_identity_is_not_a_divergence() {
        let original = fixture();
        let mut entry = original.clone();
        entry.id.push_str("-other");
        assert!(classify(&entry, DIAGNOSTIC).is_none());
        let mut entry = original.clone();
        entry.action.data.push(entry.action.data[0].clone());
        assert!(classify(&entry, DIAGNOSTIC).is_none());
        let mut entry = original.clone();
        entry.action.query = original.result_file.clone();
        assert!(classify(&entry, DIAGNOSTIC).is_none());
        let mut entry = original.clone();
        entry.action.data[0] = original.result_file.clone().unwrap();
        assert!(classify(&entry, DIAGNOSTIC).is_none());
        let mut entry = original;
        entry.result_file = entry.action.query.clone();
        assert!(classify(&entry, DIAGNOSTIC).is_none());
    }
}
