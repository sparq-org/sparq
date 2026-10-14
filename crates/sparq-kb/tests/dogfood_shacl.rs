//! Dogfood the PKG SHACL guardrails with sparq's own SHACL engine (sparq-shacl).
//!
//! sq-2m6zm.1 (epic sq-2m6zm). 🤖 SPARQ agent — dogfooding sparq as a
//! project knowledge graph. This is the FIRST real test of sparq on its own
//! knowledge: load the PKG ontology + a hand-written instance file (valid Findings/
//! Tasks + deliberately INVALID ones) and run the shapes, confirming the valid nodes
//! PASS and the invalid ones are REPORTED.
//!
//! Run: `cargo test -p sparq-kb --features validate -- --nocapture`
#![cfg(feature = "validate")]

use oxrdf::Term;
use sparq_kb::validate::validate_instances;
use sparq_kb::PKG_EXAMPLE;

/// IRIs in the example file (base `https://sparq.dev/ns/pkg/example#`).
const EX: &str = "https://sparq.dev/ns/pkg/example#";

fn focus_iris(report: &sparq_shacl::ValidationReport) -> Vec<String> {
    report
        .results
        .iter()
        .filter_map(|r| match &r.focus_node {
            Term::NamedNode(n) => Some(n.as_str().to_string()),
            _ => None,
        })
        .collect()
}

fn reported(report: &sparq_shacl::ValidationReport, local: &str) -> bool {
    let iri = format!("{EX}{local}");
    focus_iris(report).contains(&iri)
}

#[test]
fn ontology_and_shapes_parse_and_load() {
    // Both .ttl files must parse (load in sparq) — the first gate.
    let report = validate_instances(&[PKG_EXAMPLE]).expect("ontology + shapes + example load");
    // The report rendering is captured into the PR description as proof the guardrails fire.
    eprintln!(
        "=== sparq-shacl PKG guardrail report ===\n{}",
        report.to_text()
    );
}

#[test]
fn valid_nodes_pass_and_invalid_nodes_are_reported() {
    let report = validate_instances(&[PKG_EXAMPLE]).expect("validation runs");

    // The whole graph does NOT conform (it contains deliberately-invalid nodes).
    assert!(
        !report.conforms,
        "expected non-conformance: the example contains deliberately-invalid nodes"
    );

    // --- VALID nodes must NOT be reported -----------------------------------
    for ok in [
        "find-cs-dual-use",           // sourced + confident + assured + non-filler
        "find-kge-neutral",           // sourced refuting-edge finding
        "src-neumann-moerkotte-2011", // explored-status + title
        "tech-characteristic-sets",   // supersedes an existing Technique
        "task-sq-2m6zm",              // open epic, sound dep edge
        "task-sq-2m6zm-1",            // in-progress, bounded priority
    ] {
        assert!(
            !reported(&report, ok),
            "VALID node `{ok}` was wrongly reported as a violation:\n{}",
            report.to_text()
        );
    }

    // --- INVALID nodes must BE reported -------------------------------------
    // The bare Finding: missing source + confidence + assurance + filler justification.
    assert!(
        reported(&report, "find-INVALID-bare"),
        "INVALID Finding `find-INVALID-bare` was NOT caught (missing source/confidence/assurance/filler):\n{}",
        report.to_text()
    );
    // The closed-but-still-blocking task: out-of-range priority + stale-edge sh:sparql.
    assert!(
        reported(&report, "task-INVALID-closed-blocker"),
        "INVALID Task `task-INVALID-closed-blocker` was NOT caught (priority/stale-edge):\n{}",
        report.to_text()
    );
    // The out-of-enum status task.
    assert!(
        reported(&report, "task-INVALID-bad-status"),
        "INVALID Task `task-INVALID-bad-status` was NOT caught (status not in enum):\n{}",
        report.to_text()
    );
}

#[test]
fn only_valid_instances_conform() {
    // A graph with ONLY the valid subset must conform (no false positives on
    // well-formed PKG data). We slice the example down to its valid nodes.
    const VALID_ONLY: &str = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix skos:    <http://www.w3.org/2004/02/skos/core#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix fabio:   <http://purl.org/spar/fabio/> .
@prefix cito:    <http://purl.org/spar/cito/> .
@prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:      <https://sparq.dev/ns/pkg/example#> .

ex:src-ok a pkg:Source , fabio:ConferencePaper ;
  dcterms:title "A well-formed source"@en ;
  pkg:confidence 0.9 ;
  pkg:exploredStatus pkg:Explored .

ex:tech-a a pkg:Technique ; skos:prefLabel "Technique A"@en-GB ;
  dcterms:replaces ex:tech-b .
ex:tech-b a pkg:Technique ; skos:prefLabel "Technique B"@en-GB .

ex:find-ok a pkg:Finding ;
  rdfs:label "A well-formed finding"@en-GB ;
  sigimpl:justification "A genuine, sufficiently-long, non-filler justification string."@en-GB ;
  pkg:confidence 0.8 ;
  pkg:assurance secx:Claimed ;
  prov:wasDerivedFrom ex:src-ok ;
  cito:citesAsEvidence ex:src-ok .

ex:task-ok a pkg:Task ;
  dcterms:title "A well-formed open task" ;
  pkg:status pkg:Open ;
  pkg:priority 2 .
"#;

    let report = validate_instances(&[VALID_ONLY]).expect("validation runs");
    assert!(
        report.conforms,
        "a graph of ONLY well-formed PKG data must conform, but got:\n{}",
        report.to_text()
    );
}
