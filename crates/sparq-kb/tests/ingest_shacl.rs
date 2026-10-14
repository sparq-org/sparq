//! SHACL conformance gate over the **ingested** PKG graph (sq-2m6zm.2).
//!
//! sq-2m6zm.2 (epic sq-2m6zm). 🤖 SPARQ agent — PKG ingestion PoC. The
//! ingested graph (`ingest/pkg-instances.ttl`: the mechanical bd→Task projection +
//! the skills-front-matter Sources/Techniques + the hand-authored AGENTS.md Findings)
//! is run through sparq's own SHACL engine against the §2.4/§4.4 write-time
//! guardrails. The gate: **0 violations** (the guardrail working — extraction was
//! fixed until it conformed; the bd backlog's 55 stale closed→open edges are
//! guardrail-excluded by the ingest script, not silently dropped).
//!
//! Run: `cargo test -p sparq-kb --features validate --test ingest_shacl -- --nocapture`
#![cfg(feature = "validate")]

use sparq_kb::validate::validate_instances;
use sparq_kb::PKG_INSTANCES;

#[test]
fn ingested_graph_conforms_zero_violations() {
    let report = validate_instances(&[PKG_INSTANCES]).expect("ingested graph loads + validates");
    assert!(
        report.conforms && report.results.is_empty(),
        "the ingested PKG graph MUST conform with 0 SHACL violations, but got {} result(s):\n{}",
        report.results.len(),
        report.to_text()
    );
    eprintln!(
        "=== sparq-shacl PKG ingest conformance: {} violations (conforms={}) ===",
        report.results.len(),
        report.conforms
    );
}

/// The guardrail is not toothless: a stale closed→open dependency edge (the exact
/// bug the ingest script EXCLUDES, and the autonomous-scheduler bug the design cites)
/// is CAUGHT by the §4.4 TaskShape stale-edge constraint. Re-introduce one such edge
/// on top of two real tasks and confirm SHACL reports it. This proves the 0-violation
/// conformance above is the guardrail working, not the guardrail being absent.
#[test]
fn stale_edge_is_caught_by_the_guardrail() {
    // A CLOSED blocker that an OPEN task still pkg:blockedBy — must FAIL TaskShape.
    const STALE: &str = r#"
@prefix pkg: <https://sparq.dev/ns/pkg#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix kb: <https://sparq.dev/ns/pkg/kb#> .

kb:task-stale-closed a pkg:Task ;
  dcterms:title "A closed task that still blocks an open one" ;
  pkg:status pkg:Closed ; pkg:priority 1 .

kb:task-stale-open a pkg:Task ;
  dcterms:title "An open task still blocked by a closed task" ;
  pkg:status pkg:Open ; pkg:priority 1 ;
  pkg:blockedBy kb:task-stale-closed .
"#;
    let report = validate_instances(&[STALE]).expect("validation runs");
    assert!(
        !report.conforms,
        "the stale closed→open edge MUST be caught by the TaskShape guardrail"
    );
    eprintln!(
        "=== guardrail fires on a stale edge: {} violation(s) ===\n{}",
        report.results.len(),
        report.to_text()
    );
}
