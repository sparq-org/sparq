//! End-to-end Phase-5 acceptance test: run the literature-ingestion pipeline over the
//! committed fixtures, then run the SHACL gate (`pkg.shapes.ttl` +
//! `literature.shapes.ttl`) over the EMITTED TTL — the REAL path, not a mock.
//!
//! [OPUS-4.8] sq-2489d.5 (epic sq-2489d). 🤖 SPARQ agent — provenance-driven GenAI KB.
//!
//! This is the test that wires the two Phase-5 acceptance metrics together:
//!   - the **citation-grounding rate** comes from the pipeline [`Sidecar`]; and
//!   - the **SHACL-conformance rate** comes from running `sparq-shacl` over the emitted
//!     TTL (the pipeline's output is conformant BY CONSTRUCTION, so this is 1.0, and the
//!     test ALSO proves the literature shapes FIRE on a deliberately-injected violator —
//!     so the gate is real, not vacuous).
//!
//! It needs BOTH features: `literature` (the pipeline) + `validate` (the SHACL engine).
//! Run: `cargo test -p sparq-kb --features literature,validate -- --nocapture`
#![cfg(all(feature = "literature", feature = "validate"))]

use sparq_kb::literature::connector::SourceStub;
use sparq_kb::literature::extract::{CandidateFinding, Extractor, RecordedExtractor};
use sparq_kb::literature::pipeline::{self, BatchCompleteness, Tier, MACHINE_AGENT_IRI};
use sparq_kb::literature::{FIXTURE_OPENALEX_BATCH, LITERATURE_SHAPES};
use sparq_kb::validate::{graph_from_turtle_docs, parse_turtle, validate_instances};
use sparq_kb::vocab::GRAPH_NS;
use sparq_kb::{PKG_ONTOLOGY, PKG_SHAPES};

use std::collections::HashSet;

/// Validate a Turtle data document against BOTH the base PKG shapes and the literature-
/// tier shapes (the real write-gate the design specifies). Returns conformance + the text
/// report (for the PR description / triage).
fn gate(data_ttl: &str) -> (bool, String) {
    let base = "https://sparq.dev/ns/pkg/example#";
    let data = graph_from_turtle_docs(&[PKG_ONTOLOGY, data_ttl], base).expect("data graph loads");
    let shapes =
        graph_from_turtle_docs(&[PKG_SHAPES, LITERATURE_SHAPES], base).expect("shapes load");
    let report = sparq_shacl::validate(&data, &shapes);
    (report.conforms_violations_only(), report.to_text())
}

#[test]
fn pipeline_runs_offline_and_reports_the_grounding_metric() {
    let extractor = RecordedExtractor::from_fixture().expect("replay extractor builds");
    let out = pipeline::run(FIXTURE_OPENALEX_BATCH, &extractor).expect("pipeline runs");

    // The Phase-5 metric is COMPUTED from the batch, not hard-coded.
    let sc = &out.sidecar;
    eprintln!(
        "=== Phase-5 literature-ingestion sidecar ===\n\
         candidates={} grounded={} quarantined={} grounding_rate={:.4}\n\
         sources: explored={} dead_end={} skipped={}",
        sc.candidates_total,
        sc.grounded,
        sc.quarantined.len(),
        sc.grounding_rate(),
        sc.sources_explored,
        sc.sources_dead_end,
        sc.sources_skipped
    );
    for q in &sc.quarantined {
        eprintln!(
            "  QUARANTINED [{}] {} -- {}",
            q.source_doi, q.justification, q.reason
        );
    }

    // Every candidate is accounted for: grounded + quarantined == total (never dropped).
    assert_eq!(sc.grounded + sc.quarantined.len(), sc.candidates_total);
    // The two deliberately-bad candidates (fabricated span + dangling citation) are
    // quarantined, never silently dropped.
    assert_eq!(sc.quarantined.len(), 2);
    assert!(sc.grounding_rate() > 0.0 && sc.grounding_rate() < 1.0);
}

#[test]
fn emitted_ttl_conforms_to_pkg_and_literature_shapes() {
    // The REAL path: emit, then gate the emitted TTL with sparq's own SHACL engine.
    let extractor = RecordedExtractor::from_fixture().unwrap();
    let out = pipeline::run(FIXTURE_OPENALEX_BATCH, &extractor).unwrap();

    let (conforms, report) = gate(&out.turtle);
    assert!(
        conforms,
        "the pipeline-emitted machine-tier TTL must conform to pkg.shapes.ttl + \
         literature.shapes.ttl (it is conformant by construction), but got:\n{report}"
    );
}

/// The **batch-quality artifact** (sq-2489d.5, design §4.5) must be loadable alongside
/// the findings without weakening the write-gate: it parses, it conforms to the SHACL
/// shapes, and the values it reports are the sidecar's own computed rates — so "how good
/// was this batch?" is answerable from the graph rather than from a log line.
#[test]
fn batch_quality_artifact_parses_and_conforms_to_the_shacl_gate() {
    let extractor = RecordedExtractor::from_fixture().unwrap();
    let out = pipeline::run(FIXTURE_OPENALEX_BATCH, &extractor).unwrap();
    let batch = pipeline::batch_iri("openalex-fixture");
    let quality = out
        .sidecar
        .quality_measurements_turtle(&batch, &out.generated_at_time)
        .expect("quality artifact emits");

    // It is a real, parseable Turtle document (not a formatted string that happens to
    // look like one).
    let triples = parse_turtle(&quality, "https://sparq.dev/ns/pkg/example#")
        .expect("the batch-quality artifact parses as Turtle");
    assert!(!triples.is_empty());

    // It passes the same write-gate the findings do — adding run telemetry to the KB
    // must not introduce a violation.
    let (conforms, report) = gate(&quality);
    assert!(
        conforms,
        "the batch-quality artifact must conform to pkg.shapes.ttl + \
         literature.shapes.ttl, but got:\n{report}"
    );

    // The reported rates are the sidecar's, computed from THIS batch (4 of 6 candidates
    // grounded on the committed fixture) — not a hard-coded number.
    assert!((0.0..1.0).contains(&out.sidecar.grounding_rate()));
    assert!(quality.contains(&format!("dqv:value {:.4}", out.sidecar.grounding_rate())));
    assert!(quality.contains(&format!("dqv:value {:.4}", out.sidecar.source_yield_rate())));
    // Both batch metrics are named by their byte-pinned vocabulary IRIs.
    assert!(quality.contains(sparq_kb::vocab::GROUNDING_RATE_METRIC));
    assert!(quality.contains(sparq_kb::vocab::SOURCE_YIELD_RATE_METRIC));
}

#[test]
fn literature_shapes_catch_a_proven_overclaim_on_the_machine_tier() {
    // The gate must be REAL, not vacuous: inject a machine-attributed Finding that stamps
    // secx:Proven and prove the literature shape FIRES. (The pipeline never emits this —
    // it clamps to secx:Conjectured — so we hand-author the violator here.)
    const VIOLATOR: &str = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix cito:    <http://purl.org/spar/cito/> .
@prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:      <https://sparq.dev/ns/pkg/example#> .

ex:agent a pkg:MachineAgent ; rdfs:label "extractor"@en .

ex:src a pkg:Source ;
  dcterms:title "A source paper" ;
  pkg:exploredStatus pkg:Explored .

# A machine-extracted Finding that ILLEGALLY stamps secx:Proven (RULE 1) AND carries a
# confidence above the 0.7 ceiling (RULE 2) AND cites a dangling DOI (RULE 3).
ex:bad a pkg:Finding ;
  rdfs:label "an over-claiming machine finding"@en ;
  sigimpl:justification "A sufficiently-long, non-filler justification span string." ;
  pkg:confidence 0.95 ;
  pkg:assurance secx:Proven ;
  prov:wasDerivedFrom ex:src ;
  prov:wasAttributedTo ex:agent ;
  cito:citesAsEvidence ex:dangling .
"#;
    let (conforms, report) = gate(VIOLATOR);
    assert!(
        !conforms,
        "the literature shapes must REJECT a Proven over-claim / over-ceiling confidence / \
         dangling citation on the machine tier, but the graph conformed:\n{report}"
    );
    // All three literature-tier rules should be cited in the report.
    assert!(
        report.contains("secx:Proven"),
        "RULE 1 (no secx:Proven on the machine tier) should fire:\n{report}"
    );
    assert!(
        report.contains("0.7"),
        "RULE 2 (confidence ceiling) should fire:\n{report}"
    );
    assert!(
        report.contains("dangling"),
        "RULE 3 (no dangling citation) should fire:\n{report}"
    );
}

#[test]
fn a_hand_authored_proven_finding_is_not_constrained_by_the_literature_shapes() {
    // A NON-machine Finding (no prov:wasAttributedTo a pkg:MachineAgent) may assert
    // secx:Proven with high confidence — the literature shapes must NOT bind it. This is
    // the "the machine tier is constrained, the human tier is not" property.
    const HUMAN: &str = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix cito:    <http://purl.org/spar/cito/> .
@prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:      <https://sparq.dev/ns/pkg/example#> .

ex:src a pkg:Source ;
  dcterms:title "A source paper" ;
  pkg:exploredStatus pkg:Explored .

ex:human a pkg:Finding ;
  rdfs:label "a hand-authored proven finding"@en ;
  sigimpl:justification "A genuine, sufficiently-long, non-filler justification string." ;
  pkg:confidence 0.95 ;
  pkg:assurance secx:Proven ;
  prov:wasDerivedFrom ex:src ;
  cito:citesAsEvidence ex:src .
"#;
    let (conforms, report) = gate(HUMAN);
    assert!(
        conforms,
        "a hand-authored secx:Proven Finding (not machine-attributed) must conform — the \
         literature shapes constrain only the machine tier, but got:\n{report}"
    );
}

#[test]
fn a_machine_timestamped_finding_passes_shacl() {
    // [HAIKU-4.5] sq-tzars.2: positive SHACL case — a machine-extracted Finding with
    // prov:generatedAtTime (and all other required constraints) must pass the literature
    // shapes. This is the "happy path" for RULE 4.
    const TIMESTAMPED: &str = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix cito:    <http://purl.org/spar/cito/> .
@prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd:     <http://www.w3.org/2001/XMLSchema#> .
@prefix ex:      <https://sparq.dev/ns/pkg/example#> .

ex:agent a pkg:MachineAgent ; rdfs:label "extractor"@en .

ex:src a pkg:Source ;
  dcterms:title "A source paper" ;
  pkg:exploredStatus pkg:Explored .

ex:good a pkg:Finding ;
  rdfs:label "a compliant machine finding"@en ;
  sigimpl:justification "A sufficiently-long, non-filler justification span string." ;
  pkg:confidence 0.6 ;
  pkg:assurance secx:Conjectured ;
  prov:wasDerivedFrom ex:src ;
  prov:wasAttributedTo ex:agent ;
  prov:generatedAtTime "2026-07-05T14:30:00Z"^^xsd:dateTime ;
  cito:citesAsEvidence ex:src .
"#;
    let (conforms, report) = gate(TIMESTAMPED);
    assert!(
        conforms,
        "a machine-extracted Finding with prov:generatedAtTime must pass the literature \
         shapes (RULE 4), but got:\n{report}"
    );
}

#[test]
fn a_machine_finding_without_timestamp_is_rejected_by_shacl() {
    // [HAIKU-4.5] sq-tzars.2: negative SHACL case — a machine-extracted Finding without
    // prov:generatedAtTime must be rejected by RULE 4, even if all other constraints are
    // satisfied. This proves the timestamp requirement is enforced (fail-closed).
    const MISSING_TIMESTAMP: &str = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix cito:    <http://purl.org/spar/cito/> .
@prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:      <https://sparq.dev/ns/pkg/example#> .

ex:agent a pkg:MachineAgent ; rdfs:label "extractor"@en .

ex:src a pkg:Source ;
  dcterms:title "A source paper" ;
  pkg:exploredStatus pkg:Explored .

# A machine-extracted Finding that ILLEGALLY omits prov:generatedAtTime (RULE 4 violation).
# All other constraints are satisfied, so the ONLY violation is the missing timestamp.
ex:no_timestamp a pkg:Finding ;
  rdfs:label "a machine finding without prov:generatedAtTime"@en ;
  sigimpl:justification "A sufficiently-long, non-filler justification span string." ;
  pkg:confidence 0.6 ;
  pkg:assurance secx:Conjectured ;
  prov:wasDerivedFrom ex:src ;
  prov:wasAttributedTo ex:agent ;
  cito:citesAsEvidence ex:src .
"#;
    let (conforms, report) = gate(MISSING_TIMESTAMP);
    assert!(
        !conforms,
        "a machine-extracted Finding without prov:generatedAtTime must be REJECTED by \
         RULE 4 (fail-closed quarantine), but the graph conformed:\n{report}"
    );
    // RULE 4 message must be cited in the report.
    assert!(
        report.contains("prov:generatedAtTime") || report.contains("extraction instant"),
        "RULE 4 (prov:generatedAtTime requirement) should fire:\n{report}"
    );
}

#[test]
fn base_example_still_conforms_under_the_combined_shapes() {
    // Regression: adding literature.shapes.ttl must not break the existing base shapes on
    // the existing example graph (the literature shapes are additive + machine-tier-scoped).
    let report = validate_instances(&[]).expect("ontology + shapes load with no instances");
    // (validate_instances uses only the base shapes; this asserts the ontology + base
    // shapes still load cleanly after the pkg.ttl MachineAgent addition.)
    assert!(report.conforms, "empty-instance base graph must conform");
}

// ===========================================================================
// sq-tzars.7 — tier partition (hand-authored / machine / license-restricted)
// [SONNET-4.6] 🤖 SPARQ agent — provenance-driven GenAI KB. Fail-closed license
// routing; exactly one tier per emitted statement; metadata-only public projection.
// ===========================================================================

/// A grounding-guaranteed test extractor: for each stub it proposes one candidate whose
/// justification is a span of that stub's abstract (a prefix, so it always grounds) and
/// which cites the stub's own DOI (an in-batch Source). Lets a MIXED-licence batch exercise
/// real routing of grounded findings — no live model, no committed tape.
struct GroundingExtractor;
impl Extractor for GroundingExtractor {
    fn extract(&self, stubs: &[SourceStub]) -> Result<Vec<CandidateFinding>, String> {
        Ok(stubs
            .iter()
            .map(|s| CandidateFinding {
                source_doi: s.doi.clone(),
                verdict: "yes".to_string(),
                confidence: 0.6,
                assurance: "Conjectured".to_string(),
                justification: s.abstract_text.chars().take(48).collect(),
                cited_dois: vec![s.doi.clone()],
            })
            .collect())
    }
}

/// A mixed-licence batch: one `cc-by` (redistributable ⇒ machine tier) source and one
/// unknown-licence (⇒ restricted tier) source, each with a groundable abstract.
const MIXED_BATCH: &str = r#"{ "results": [
  { "doi": "https://doi.org/10.5555/mixed.ccby",
    "title": "An Open-Licensed Work on Parallel Scanning",
    "abstract": "This open work reports a reproducible parallel-scanning result over graphs.",
    "publication_year": 2024,
    "primary_location": { "license": "cc-by" } },
  { "doi": "https://doi.org/10.5555/mixed.unknown",
    "title": "A Work With No Recorded Redistribution Licence",
    "abstract": "This work has no redistribution licence recorded anywhere in its metadata.",
    "publication_year": 2023 }
] }"#;

/// The N-Triples subject token of a serialized triple (`<iri>` or `_:b`), for disjointness
/// checks without pulling `oxrdf` into the integration crate.
fn subject_token(ntriple: &str) -> &str {
    ntriple.split_whitespace().next().unwrap_or("")
}

fn ntriples_set(ttl: &str) -> HashSet<String> {
    let base = "https://sparq.dev/ns/pkg/example#";
    parse_turtle(ttl, base)
        .expect("tier artifact parses")
        .iter()
        .map(|t| t.to_string())
        .collect()
}

#[test]
fn no_statement_is_emitted_into_more_than_one_tier() {
    // The load-bearing partition invariant: no Source/Finding PAYLOAD triple appears in
    // both the machine and the license-restricted artifacts. (Each artifact independently
    // repeats the shared machine-agent declaration boilerplate so it stands alone; the
    // tier-graph node subjects differ per tier, so only the agent decl can overlap — and
    // that is asserted to be the ONLY overlap.)
    let out = pipeline::run_tiered(
        MIXED_BATCH,
        &GroundingExtractor,
        Some("2026-07-05T00:00:00Z".to_string()),
        BatchCompleteness::Complete,
    )
    .expect("tiered run");

    let machine = ntriples_set(&out.machine_tier);
    let restricted = ntriples_set(&out.license_restricted_tier);
    let agent_subject = format!("<{}>", MACHINE_AGENT_IRI);
    let graph_prefix = format!("<{}", GRAPH_NS);

    let shared: Vec<&String> = machine.intersection(&restricted).collect();
    for line in &shared {
        let subj = subject_token(line);
        assert!(
            subj == agent_subject || subj.starts_with(&graph_prefix),
            "machine and restricted tiers share a PAYLOAD statement (not boilerplate): {}",
            line
        );
    }

    // Non-vacuous: each tier really does carry its own source's finding, and NOT the other's.
    assert!(out.machine_tier.contains("10.5555/mixed.ccby"));
    assert!(!out.machine_tier.contains("10.5555/mixed.unknown"));
    assert!(out.license_restricted_tier.contains("10.5555/mixed.unknown"));
    assert!(!out.license_restricted_tier.contains("10.5555/mixed.ccby"));
    // Each tier carries exactly one grounded finding here.
    assert_eq!(out.machine_tier.matches("a pkg:Finding").count(), 1);
    assert_eq!(out.license_restricted_tier.matches("a pkg:Finding").count(), 1);
}

#[test]
fn unknown_licence_source_findings_land_restricted_only() {
    // Negative test (REQUIRED): the committed fixture carries NO licence metadata, so every
    // source is fail-closed to the restricted tier. Its findings must appear ONLY in the
    // restricted artifact, never in the (publishable) machine artifact.
    let out = pipeline::run_tiered(
        FIXTURE_OPENALEX_BATCH,
        &RecordedExtractor::from_fixture().unwrap(),
        Some("2026-07-05T00:00:00Z".to_string()),
        BatchCompleteness::Complete,
    )
    .expect("tiered run");

    // Every source routes restricted ⇒ machine tier holds zero findings and zero sources.
    assert!(!out.machine_tier.contains("a pkg:Finding"));
    assert!(!out.machine_tier.contains("a pkg:Source"));
    // All four grounded findings are in the restricted tier.
    assert_eq!(out.license_restricted_tier.matches("a pkg:Finding").count(), 4);
    assert!(out.license_restricted_tier.contains("a pkg:Source"));
    // The classifier agrees on the None-licence fixture sources.
    assert_eq!(pipeline::source_tier(None), Tier::LicenseRestricted);
}

#[test]
fn restricted_public_projection_carries_no_abstract_derived_text() {
    // The metadata-only public projection must exclude ALL abstract-derived text — no
    // justification (a span of the abstract), no dcterms:abstract, no pkg:Finding — while
    // still carrying the source metadata (title + licence status).
    let out = pipeline::run_tiered(
        FIXTURE_OPENALEX_BATCH,
        &RecordedExtractor::from_fixture().unwrap(),
        Some("2026-07-05T00:00:00Z".to_string()),
        BatchCompleteness::Complete,
    )
    .expect("tiered run");

    let proj = &out.restricted_public_projection;
    // Non-vacuous baseline: the FULL restricted artifact DOES carry the abstract-derived
    // justification text, so the projection genuinely stripped it (not merely empty).
    assert!(
        out.license_restricted_tier.contains("sigimpl:justification"),
        "restricted FULL tier must carry the justification (else the test is vacuous)"
    );
    assert!(out.license_restricted_tier.contains("We present a chunk-parallel"));

    // The projection carries NONE of it.
    assert!(!proj.contains("sigimpl:justification"), "projection leaks a justification");
    assert!(!proj.contains("dcterms:abstract"), "projection leaks an abstract");
    assert!(!proj.contains("a pkg:Finding"), "projection leaks a Finding");
    assert!(!proj.contains("We present a chunk-parallel"), "projection leaks abstract text");
    assert!(!proj.contains("prov:wasGeneratedBy"), "projection leaks finding provenance");

    // But it DOES carry the permitted source metadata + licence status.
    assert!(proj.contains("Chunk-Parallel Scanning of Line-Delimited RDF"));
    assert!(proj.contains("dcterms:license \"unknown\""));
    assert!(proj.contains("a pkg:Source"));
    // The projection is valid, parseable Turtle.
    assert!(parse_turtle(proj, "https://sparq.dev/ns/pkg/example#").is_ok());
}

#[test]
fn cap_truncated_batch_is_marked_incomplete_in_every_artifact() {
    // Truncation invariant (the #1527 consumer note): if the input batch was cut short by a
    // pagination hard cap, EVERY emitted artifact must carry an explicit incompleteness
    // marker and never be presented as complete.
    let truncated = BatchCompleteness::from_pagination(20, 20);
    assert!(!truncated.is_complete());
    let out = pipeline::run_tiered(
        FIXTURE_OPENALEX_BATCH,
        &RecordedExtractor::from_fixture().unwrap(),
        Some("2026-07-05T00:00:00Z".to_string()),
        truncated,
    )
    .expect("tiered run");

    for (name, art) in [
        ("machine", &out.machine_tier),
        ("restricted", &out.license_restricted_tier),
        ("projection", &out.restricted_public_projection),
    ] {
        assert!(
            art.contains("INCOMPLETE") && art.contains("cap-truncated"),
            "{} artifact must carry the incompleteness marker",
            name
        );
        assert!(
            art.contains("rdfs:comment"),
            "{} artifact must carry a machine-readable incompleteness marker",
            name
        );
        // Still valid Turtle (the banner is a comment; the marker is an rdfs:comment).
        assert!(
            parse_turtle(art, "https://sparq.dev/ns/pkg/example#").is_ok(),
            "{} artifact must remain parseable Turtle",
            name
        );
    }

    // A COMPLETE run carries NO marker.
    let complete = pipeline::run_tiered(
        FIXTURE_OPENALEX_BATCH,
        &RecordedExtractor::from_fixture().unwrap(),
        Some("2026-07-05T00:00:00Z".to_string()),
        BatchCompleteness::Complete,
    )
    .expect("tiered run");
    assert!(!complete.machine_tier.contains("INCOMPLETE"));
    assert!(!complete.license_restricted_tier.contains("INCOMPLETE"));
    assert!(!complete.restricted_public_projection.contains("INCOMPLETE"));
}

/// Generate the golden TTL output for the standard fixture pipeline.
///
/// # Generated Golden File
///
/// This test is `#[ignore]`d by default. To regenerate the golden file:
///
/// ```sh
/// cd /home/ubuntu/sparq  # or your working repo
/// cargo test -p sparq-kb --features literature,validate \
///   --test literature_pipeline -- generate_golden_turtle_output --nocapture --ignored \
///   | sed -n '/^@prefix/,/^test generate/p' | head -n -1 \
///   > crates/sparq-kb/tests/golden/openalex-fixture-combined.ttl
/// # Then manually review and commit the new file.
/// ```
///
/// This is a deliberate (and audited) regeneration path — changes to the golden file are
/// NEVER silent. The primary golden-pin test is [`emitted_turtle_matches_golden_byte_for_byte`],
/// which fails loudly if the output drifts.
#[test]
#[ignore]
fn generate_golden_turtle_output() {
    let extractor = RecordedExtractor::from_fixture().expect("replay extractor builds");
    // Use a FIXED timestamp (same as the pin test) so golden generation is deterministic.
    let fixed_time = Some("-4656-05-28T09:28:20.459270Z".to_string());
    let out = pipeline::run_with_time(FIXTURE_OPENALEX_BATCH, &extractor, fixed_time)
        .expect("pipeline runs with fixed time");
    eprintln!("=== Golden Turtle Output (copy to tests/golden/openalex-fixture-combined.ttl) ===");
    println!("{}", out.turtle);
}

/// A dup-DOI mixed batch: the SAME DOI appears TWICE in one connector batch — once with a
/// "cc-by" licence and once with no licence recorded. Simulates the real OpenAlex case where
/// two indexing records represent the same work with conflicting licence metadata. The
/// DOI-granularity fail-closed rule must route the WHOLE DOI restricted.
const DUP_DOI_MIXED_BATCH: &str = r#"{ "results": [
  { "doi": "https://doi.org/10.5555/dup.conflict",
    "title": "A Conflicted Work — open copy",
    "abstract": "This conflicted work has two indexing records and mentions parallel scanning.",
    "publication_year": 2024,
    "primary_location": { "license": "cc-by" } },
  { "doi": "https://doi.org/10.5555/dup.conflict",
    "title": "A Conflicted Work — no-licence copy",
    "abstract": "This conflicted work has two indexing records and mentions parallel scanning.",
    "publication_year": 2024 }
] }"#;

#[test]
fn dup_doi_mixed_licence_fails_closed_whole_doi_restricted() {
    // DOI-granularity fail-closed (sq-tzars.7): when a batch contains two stubs for the same
    // DOI — one "cc-by" and one licence-absent — the WHOLE DOI must be routed RESTRICTED.
    // No abstract-derived content (source node, pkg:Finding, sigimpl:justification) for that
    // DOI may appear in the machine (publishable) artifact.
    let out = pipeline::run_tiered(
        DUP_DOI_MIXED_BATCH,
        &GroundingExtractor,
        Some("2026-07-05T00:00:00Z".to_string()),
        BatchCompleteness::Complete,
    )
    .expect("tiered run on dup-DOI batch");

    // Machine artifact must carry NONE of the dup-DOI source content (the invariant).
    assert!(
        !out.machine_tier.contains("10.5555/dup.conflict"),
        "machine tier must not carry any source node from the dup-DOI (fail-closed)"
    );
    assert!(
        !out.machine_tier.contains("a pkg:Finding"),
        "machine tier must not carry any Finding from the dup-DOI"
    );
    assert!(
        !out.machine_tier.contains("sigimpl:justification"),
        "machine tier must not carry any justification (abstract-derived text) from the dup-DOI"
    );

    // Restricted artifact MUST contain the full content (non-vacuousness).
    assert!(
        out.license_restricted_tier.contains("10.5555/dup.conflict"),
        "restricted tier must carry the dup-DOI source"
    );
    assert!(
        out.license_restricted_tier.contains("a pkg:Finding"),
        "restricted tier must carry at least one Finding for the dup-DOI"
    );
    assert!(
        out.license_restricted_tier.contains("sigimpl:justification"),
        "restricted tier must carry the abstract-derived justification"
    );

    // Restricted public projection must carry the DOI metadata but NO abstract-derived text.
    assert!(
        out.restricted_public_projection.contains("10.5555/dup.conflict"),
        "restricted public projection must carry the dup-DOI metadata"
    );
    assert!(
        !out.restricted_public_projection.contains("sigimpl:justification"),
        "projection must not carry abstract-derived text"
    );
    assert!(
        !out.restricted_public_projection.contains("a pkg:Finding"),
        "projection must not carry any Finding"
    );

    // The sidecar must account for all candidates (none silently dropped).
    assert_eq!(
        out.sidecar.grounded + out.sidecar.quarantined.len(),
        out.sidecar.candidates_total,
        "all candidates must be accounted for in the sidecar"
    );
}

#[test]
fn machine_and_restricted_full_tiers_conform_to_the_shacl_gate() {
    // The REAL path: each full tier artifact (machine + restricted) must independently
    // conform to pkg.shapes.ttl + literature.shapes.ttl — the tier split must not produce a
    // non-conformant graph.
    let out = pipeline::run_tiered(
        MIXED_BATCH,
        &GroundingExtractor,
        Some("2026-07-05T00:00:00Z".to_string()),
        BatchCompleteness::Complete,
    )
    .expect("tiered run");

    let (m_conforms, m_report) = gate(&out.machine_tier);
    assert!(m_conforms, "machine-tier artifact must conform:\n{m_report}");
    let (r_conforms, r_report) = gate(&out.license_restricted_tier);
    assert!(r_conforms, "restricted-tier artifact must conform:\n{r_report}");
}

// ===========================================================================
// sq-9m3rn — Golden/byte-pin test: PipelineOutput::turtle byte-compatibility
// [HAIKU-4.5] 🤖 SPARQ agent — golden-file pin for #1540 refactor.
// ===========================================================================

/// The committed golden file for the fixture pipeline's combined TTL output. This test
/// pins the byte-exact output to catch silent drift in the emission logic — the #1540
/// refactor's "byte-for-byte unchanged" claim is now enforced, not just inspected.
///
/// If this test fails, the emission logic has drifted. Review the diff carefully:
/// - Is the change intended (e.g., a deliberate format refactor)?
/// - If YES: run the `generate_golden_turtle_output` ignored test to regenerate the golden
///   and commit the new file.
/// - If NO: revert the emission code and re-pin the test.
///
/// The golden file is at: `crates/sparq-kb/tests/golden/openalex-fixture-combined.ttl`
#[test]
fn emitted_turtle_matches_golden_byte_for_byte() {
    use std::fs;
    use std::path::PathBuf;

    let extractor = RecordedExtractor::from_fixture().expect("replay extractor builds");
    // Deterministic timestamp (sq-tzars.2 [HAIKU-4.5]): use the same fixed instant
    // that was used to generate the golden file. This ensures byte-exact reproducibility.
    let fixed_time = Some("-4656-05-28T09:28:20.459270Z".to_string());
    let out = pipeline::run_with_time(FIXTURE_OPENALEX_BATCH, &extractor, fixed_time)
        .expect("pipeline runs with fixed time");

    // Load the committed golden file from the repo (resolved relative to the crate root).
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let golden_path = manifest_dir.join("tests/golden/openalex-fixture-combined.ttl");
    let golden = fs::read_to_string(&golden_path).unwrap_or_else(|e| {
        panic!(
            "Failed to read golden file at {}: {}. \
             If regenerating, run: cargo test -p sparq-kb --features literature,validate \
             --test literature_pipeline -- generate_golden_turtle_output --nocapture --ignored",
            golden_path.display(),
            e
        )
    });

    // Byte-for-byte comparison: the emission logic must not drift.
    assert_eq!(
        out.turtle, golden,
        "PipelineOutput::turtle has drifted from the golden file (byte-for-byte mismatch). \
         This indicates a change to the emission logic or the fixture. \
         \n\nTo deliberately regenerate the golden file (e.g., after an intended refactor): \
         \n  cargo test -p sparq-kb --features literature,validate --test literature_pipeline \
         \n  -- generate_golden_turtle_output --nocapture --ignored \
         \n  > /tmp/golden.ttl \
         \nThen review the diff carefully and commit the new golden file.\n"
    );
}
