//! Integration test for the PKG **natural-language tool** envelope (agent flavor) over
//! the REAL ingested PKG — sq-ve5dy. The soundness guarantee (every answer is returned
//! with the executed SPARQL + resolved IRIs + grounding confidence, so the caller can
//! verify it was computed, not guessed) must hold against the actual data, not just a
//! tiny fixture.
//!
//! sq-ve5dy (epic sq-2m6zm). 🤖 SPARQ agent — dogfooding sparq as a project
//! knowledge graph. Written while Fable unavailable; flag for re-review when Fable returns.
//!
//! Run: `cargo test -p sparq-kb --features query --test nl_tool -- --nocapture`.
#![cfg(feature = "query")]

use sparq_kb::query::load_pkg;
use sparq_kb::query::nl_tool::{self, Confidence};

#[test]
fn canned_query_over_real_pkg_is_a_canned_confidence_envelope() {
    // The introspect "schema-classes" canned query over the real ingest: the answer is
    // computed (Task dominates the projection), the executed SPARQL is echoed for
    // verification, and the confidence is `Canned` (curated template).
    let g = load_pkg().expect("PKG loads");
    let r = nl_tool::run_canned(&g, "schema-classes", None).expect("runs");
    assert_eq!(r.confidence, Confidence::Canned);
    assert!(
        r.row_count >= 4,
        "expected the PKG class card; got {}",
        r.row_count
    );
    // The executed SPARQL is the real canned text, verifiable by re-running.
    assert!(r.executed_sparql.contains("GROUP BY ?class"));
    // The answer is the engine's computed rows (Task is the largest class).
    assert!(
        r.answer.to_lowercase().contains("task"),
        "answer should mention the Task class; got: {}",
        r.answer
    );
    // rdf:type is the predicate the query relied on — surfaced as a resolved IRI.
    assert!(r
        .resolved_iris
        .contains(&"http://www.w3.org/1999/02/22-rdf-syntax-ns#type".to_string()));
    assert!(r.ungrounded_iris.is_empty(), "{:?}", r.ungrounded_iris);
}

#[test]
fn parameterised_canned_query_substitutes_and_runs() {
    // `task-blocks` for sq-8thu (the most-depended-on blocker) returns many downstream
    // dependents; the substituted arg is visible in the echoed SPARQL.
    let g = load_pkg().expect("PKG loads");
    let r = nl_tool::run_canned(&g, "task-blocks", Some("sq-8thu")).expect("runs");
    assert_eq!(r.confidence, Confidence::Canned);
    assert!(
        r.row_count >= 10,
        "sq-8thu should have many downstream dependents; got {}",
        r.row_count
    );
    assert!(r.executed_sparql.contains("sq-8thu"));
}

#[test]
fn raw_grounded_query_over_real_pkg_is_grounded_with_no_hints() {
    // A raw count over the real Task class: every term grounds, confidence `Grounded`.
    let g = load_pkg().expect("PKG loads");
    let r = nl_tool::run_raw(
        &g,
        "PREFIX pkg: <https://sparq.dev/ns/pkg#>\n\
         SELECT (COUNT(?t) AS ?n) WHERE { ?t a pkg:Task }",
    )
    .expect("runs");
    assert_eq!(r.confidence, Confidence::Grounded);
    assert!(r.ungrounded_iris.is_empty());
    assert!(r.hints.is_empty());
    assert_eq!(r.row_count, 1);
}

#[test]
fn raw_ungrounded_query_lowers_confidence_and_surfaces_provenance() {
    // A plausible-but-wrong predicate (`pkg:dependsUpon`, not the real `pkg:dependsOn`):
    // the tool does NOT silently answer — it flags the ungrounded term, lowers the
    // confidence, and surfaces same-namespace hints so the agent can re-ground.
    let g = load_pkg().expect("PKG loads");
    let r = nl_tool::run_raw(
        &g,
        "PREFIX pkg: <https://sparq.dev/ns/pkg#>\n\
         SELECT ?a ?b WHERE { ?a pkg:dependsUpon ?b }",
    )
    .expect("the query is valid, just ungrounded");
    assert_eq!(r.confidence, Confidence::Ungrounded);
    assert_eq!(
        r.ungrounded_iris,
        vec!["https://sparq.dev/ns/pkg#dependsUpon".to_string()]
    );
    // The real `pkg:dependsOn` predicate is in the same namespace, so it surfaces as a
    // repair hint.
    assert!(
        r.hints
            .iter()
            .any(|h| h == "https://sparq.dev/ns/pkg#dependsOn"),
        "expected pkg:dependsOn hint; got {:?}",
        r.hints
    );
    // Honest "none" answer, computed not guessed.
    assert_eq!(r.row_count, 0);
}

#[test]
fn json_envelope_round_trips_over_real_pkg() {
    // The JSON the agent-flavor sub-agent emits must parse back identical (the wire
    // contract between the cheap-model sub-agent and the orchestrator).
    let g = load_pkg().expect("PKG loads");
    let r = nl_tool::run_canned(&g, "finding-provenance", None).expect("runs");
    let json = r.to_json();
    let back: nl_tool::NlToolResult = serde_json::from_str(&json).expect("round-trips");
    assert_eq!(back, r);
    // Findings exist in the ingest, so the answer is non-empty + sourced.
    assert!(
        r.row_count >= 6,
        "expected the AGENTS.md finding slice; got {}",
        r.row_count
    );
}

#[test]
fn unknown_canned_name_is_an_error_not_a_silent_guess() {
    let g = load_pkg().expect("PKG loads");
    let err = nl_tool::run_canned(&g, "no-such-canned-query", None).unwrap_err();
    assert!(err.contains("no canned query named"), "{err}");
}
