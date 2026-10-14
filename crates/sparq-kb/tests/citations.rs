//! Integration tests for the PKG-native citation renderer (sq-2489d.11).
//!
//! Exercises the REAL canned-query → render path over both a curated fixture
//! KB (known provenance, known citation count) and the full ingested PKG
//! instance graph.  The tests assert the three bead-acceptance criteria:
//!
//! 1. A canned-query answer path renders deriving sources as `[source]`
//!    citations from `prov:wasDerivedFrom`.
//! 2. The metric harness reports citation-resolution-rate (target 1.0) and
//!    fabricated-citation count (target 0).
//! 3. A Finding with NO `prov:wasDerivedFrom` renders NO fabricated citation.
//!
//! sq-2489d.11. 🤖 SPARQ agent — citation renderer for the PKG-native tier.
//!
//! Run: `cargo test -p sparq-kb --features query --test citations`
#![cfg(feature = "query")]

use sparq_kb::query::citations::render_citations;
use sparq_kb::query::{ask_pkg, load_pkg, load_pkg_with_extra};
use sparq_kb::query::canned;

// ---------------------------------------------------------------------------
// Fixture KB: two Findings with provenance + one without.
//
// kb:src-a and kb:src-b are REAL Sources present in the fixture graph;
// both Findings with prov:wasDerivedFrom resolve correctly (rate 1.0).
// kb:f-no-prov has no prov:wasDerivedFrom and is excluded by the mandatory
// join in FINDING_PROVENANCE.
// ---------------------------------------------------------------------------
const FIXTURE_KB: &str = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .
@prefix kb:      <https://sparq.dev/ns/pkg/kb#> .

kb:src-a a pkg:Source ;
    rdfs:label "Source A"@en-GB ;
    pkg:confidence 0.9 .

kb:src-b a pkg:Source ;
    rdfs:label "Source B"@en-GB ;
    pkg:confidence 0.8 .

kb:f-alpha a pkg:Finding ;
    rdfs:label "Alpha finding"@en-GB ;
    sigimpl:justification "A sufficiently long non-filler justification for alpha"@en-GB ;
    prov:wasDerivedFrom kb:src-a ;
    dcterms:source "AGENTS.md#alpha-section" ;
    pkg:assurance secx:Claimed ;
    pkg:confidence 0.88 .

kb:f-beta a pkg:Finding ;
    rdfs:label "Beta finding"@en-GB ;
    sigimpl:justification "A sufficiently long non-filler justification for beta"@en-GB ;
    prov:wasDerivedFrom kb:src-b ;
    dcterms:source "AGENTS.md#beta-section" ;
    pkg:assurance secx:Claimed ;
    pkg:confidence 0.75 .

kb:f-no-prov a pkg:Finding ;
    rdfs:label "Gamma finding — no provenance"@en-GB ;
    sigimpl:justification "A justification for the provenance-free finding"@en-GB ;
    pkg:assurance secx:Conjectured ;
    pkg:confidence 0.5 .
"#;

// ---------------------------------------------------------------------------
// (1) Positive test: a fixture KB with known provenance → citations render
//     correctly, metrics are sound (rate 1.0, fabricated 0).
// ---------------------------------------------------------------------------

#[test]
fn fixture_kb_renders_correct_citations_and_metrics_are_sound() {
    let graph = load_pkg_with_extra(&[FIXTURE_KB]).expect("PKG + fixture loads");
    let rows = ask_pkg(&graph, &canned::FINDING_PROVENANCE.render(None))
        .expect("FINDING_PROVENANCE query runs");

    // The fixture has two Findings with prov:wasDerivedFrom (f-alpha, f-beta).
    // f-no-prov has none and is excluded by the mandatory join.
    assert!(
        rows.rows.len() >= 2,
        "at least the two fixture Findings with provenance must appear; got {} rows",
        rows.rows.len()
    );

    // Collect the fixture-specific rows (label contains "finding").
    let fixture_rows: Vec<_> = rows
        .rows
        .iter()
        .filter(|r| {
            matches!(&r[0], Some(oxrdf::Term::Literal(l)) if
                l.value().contains("Alpha finding") || l.value().contains("Beta finding"))
        })
        .collect();
    assert_eq!(
        fixture_rows.len(),
        2,
        "both Alpha and Beta fixture Findings must be in the result"
    );

    let report = render_citations(&graph, &rows);

    // (a) Resolution-rate 1.0, fabricated-count 0.
    assert_eq!(
        report.metrics.fabricated_count,
        0,
        "no fabricated citations: every source IRI in the result must be in the graph"
    );
    #[allow(clippy::float_cmp)]
    {
        assert_eq!(
            report.metrics.citation_resolution_rate(),
            1.0,
            "citation-resolution-rate must be 1.0 on the fixture"
        );
    }

    // (b) The rendered text contains [source] citations for both fixture Findings.
    assert!(
        report.rendered_text.contains("[source: src-a]"),
        "rendered text missing [source: src-a]: {}",
        report.rendered_text
    );
    assert!(
        report.rendered_text.contains("[source: src-b]"),
        "rendered text missing [source: src-b]: {}",
        report.rendered_text
    );

    // (c) Section anchors are present.
    assert!(
        report.rendered_text.contains("AGENTS.md#alpha-section")
            || report.rendered_text.contains("AGENTS.md#beta-section"),
        "rendered text must include at least one section anchor: {}",
        report.rendered_text
    );

    // (d) The no-provenance Finding does NOT appear.
    assert!(
        !report.rendered_text.contains("Gamma finding"),
        "the Finding without provenance must not appear in the output: {}",
        report.rendered_text
    );
}

// ---------------------------------------------------------------------------
// (2) Negative test: a Finding with NO prov:wasDerivedFrom produces no
//     fabricated [source] citation in the rendered output.
// ---------------------------------------------------------------------------

#[test]
fn finding_without_prov_produces_no_fabricated_citation_in_output() {
    let graph = load_pkg_with_extra(&[FIXTURE_KB]).expect("PKG + fixture loads");
    let rows = ask_pkg(&graph, &canned::FINDING_PROVENANCE.render(None))
        .expect("query runs");

    // The mandatory prov:wasDerivedFrom join excludes f-no-prov.
    assert!(
        !rows.rows.iter().any(|r| {
            matches!(&r[0], Some(oxrdf::Term::Literal(l)) if l.value().contains("Gamma"))
        }),
        "the no-provenance Finding must not appear in the FINDING_PROVENANCE result set"
    );

    let report = render_citations(&graph, &rows);

    // No dangling/placeholder citation for the excluded Finding.
    assert!(
        !report.rendered_text.contains("Gamma"),
        "the no-provenance Finding must not appear in the citation output: {}",
        report.rendered_text
    );
    assert!(
        !report.rendered_text.contains("[source: none]"),
        "no [source: none] placeholder must appear (the row was excluded by the join): {}",
        report.rendered_text
    );
    assert_eq!(
        report.metrics.fabricated_count,
        0,
        "fabricated-count must be 0"
    );
}

// ---------------------------------------------------------------------------
// (3) Real-PKG metric test: run FINDING_PROVENANCE over the full ingested
//     instances and assert citation-resolution-rate 1.0 + fabricated 0.
//     This is the live load-bearing invariant check.
// ---------------------------------------------------------------------------

#[test]
fn real_pkg_citations_resolution_rate_is_one_and_fabricated_count_is_zero() {
    let graph = load_pkg().expect("full ingested PKG loads");
    let rows = ask_pkg(&graph, &canned::FINDING_PROVENANCE.render(None))
        .expect("FINDING_PROVENANCE query runs over real PKG");

    // The real PKG has the AGENTS.md Findings tier (at least 6 Findings per
    // the existing query_canned assertion).
    assert!(
        rows.rows.len() >= 6,
        "expected the AGENTS.md finding slice in the real PKG; got {} rows",
        rows.rows.len()
    );

    let report = render_citations(&graph, &rows);

    // Load-bearing invariant: every [source] citation resolves to a real
    // prov:wasDerivedFrom triple in the graph.
    assert_eq!(
        report.metrics.fabricated_count,
        0,
        "fabricated-citation count must be 0 over the real PKG \
         (SHACL already forbids dangling cito:citesAsEvidence, \
         but the harness MEASURES, not assumes)"
    );
    #[allow(clippy::float_cmp)]
    {
        assert_eq!(
            report.metrics.citation_resolution_rate(),
            1.0,
            "citation-resolution-rate must be 1.0 over the real PKG"
        );
    }

    // Every cited Finding's rendered line contains a [source: ...] tag.
    for line in report.rendered_text.lines() {
        if !line.trim().is_empty() {
            assert!(
                line.contains("[source:"),
                "every rendered line must carry a [source:] citation tag: {line:?}"
            );
        }
    }

    // The total citation count equals the row count (one citation per row).
    assert_eq!(
        report.metrics.total_citations,
        rows.rows.len(),
        "total_citations must equal the number of result rows"
    );
}
