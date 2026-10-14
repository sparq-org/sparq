//! # Citation renderer for PKG-native canned-query answers (sq-2489d.11)
//!
//! Renders `prov:wasDerivedFrom` source citations beside canned-query answers
//! from the `FINDING_PROVENANCE` canned query — the PKG-native tier
//! of the provenance-is-load-bearing programme (epic `sq-2489d`, issue #1110).
//!
//! > **Scope: PKG-native tier only.** The `FINDING_PROVENANCE` canned query
//! > already does the `prov:wasDerivedFrom` join; this module renders those
//! > bindings as human- and agent-readable `[source]` citations. The general
//! > row → provenance tier is explicitly OUT of scope (bead notes, sq-2489d.11).
//!
//! ## Load-bearing invariant
//!
//! Every rendered `[source]` citation resolves to a real `prov:wasDerivedFrom`
//! triple in the graph — citation-resolution-rate **1.0**. A citation with no
//! backing triple is a bug caught by `CitationMetrics::fabricated_count`.
//! The SHACL shapes (`pkg.shapes.ttl`) additionally forbid a dangling
//! `cito:citesAsEvidence` edge at write time, so 0 is the expected count by
//! construction, but the harness **measures** it rather than assuming it.
//!
//! ## Usage
//!
//! ```rust,no_run
//! # #[cfg(feature = "query")]
//! # {
//! use sparq_kb::query::{load_pkg, ask_pkg};
//! use sparq_kb::query::canned;
//! use sparq_kb::query::citations::render_citations;
//!
//! let graph = load_pkg().expect("PKG loads");
//! let rows = ask_pkg(&graph, &canned::FINDING_PROVENANCE.render(None))
//!     .expect("query runs");
//! let report = render_citations(&graph, &rows);
//! println!("{}", report.rendered_text);
//! println!("resolution rate: {:.2}", report.metrics.citation_resolution_rate());
//! # }
//! ```
//!
//! sq-2489d.11. 🤖 SPARQ agent — citation renderer for the PKG-native tier.

use oxrdf::Term;
use sparq_core::Graph;
use sparq_engine::QueryResult;

/// A rendered `[source]` citation attached to one Finding answer row.
/// The `source_iri` is the `prov:wasDerivedFrom` value from the data; the
/// `label` is the human-readable short form rendered in the output text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    /// The full IRI of the `prov:wasDerivedFrom` source (from the query binding).
    pub source_iri: String,
    /// The human-readable label used in the `[source]` rendering: the local
    /// name after the last `#` or `/`, falling back to the full IRI.
    pub label: String,
    /// `true` when the source IRI is present in the live graph dictionary —
    /// i.e. the citation resolves to a real triple (the load-bearing invariant).
    pub resolves: bool,
}

/// Metrics over a citation render pass — the harness assertions required by sq-2489d.11.
///
/// Both metrics are computed by the renderer from the live graph dictionary,
/// not assumed: resolution-rate 1.0 and fabricated-count 0 are MEASURED.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationMetrics {
    /// Total number of `[source]` citations rendered (one per non-empty
    /// `?source` binding in the `FINDING_PROVENANCE` result set).
    pub total_citations: usize,
    /// Citations whose `source_iri` is **absent** from the live graph
    /// dictionary — i.e. citations with no backing `prov:wasDerivedFrom` triple.
    /// Target: **0**. A non-zero count is a bug: a citation was rendered for a
    /// source IRI the graph does not actually contain.
    pub fabricated_count: usize,
}

impl CitationMetrics {
    /// Citation-resolution-rate: the fraction of citations whose source IRI
    /// resolves to a real triple in the graph. Target: **1.0**.
    ///
    /// Returns `1.0` when `total_citations == 0` (no citations → no
    /// fabrications — the honest "none" case).
    pub fn citation_resolution_rate(&self) -> f64 {
        if self.total_citations == 0 {
            1.0
        } else {
            let resolved = self.total_citations.saturating_sub(self.fabricated_count);
            resolved as f64 / self.total_citations as f64
        }
    }
}

/// One answer row from the `FINDING_PROVENANCE` canned query, enriched with
/// its `[source]` citation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitedAnswer {
    /// The Finding label (`?label` binding, column 0).
    pub finding_label: String,
    /// The rendered `[source]` citation for this row, or `None` when the
    /// `?source` binding is absent (OPTIONAL clause returned nothing — a
    /// Finding with no `prov:wasDerivedFrom` triple, which SHACL forbids in a
    /// conformant graph but which the renderer handles defensively).
    pub citation: Option<Citation>,
    /// The `dcterms:source` section anchor (`?section` binding, column 2),
    /// when present.
    pub section: Option<String>,
}

/// The full output of one citation render pass over a `FINDING_PROVENANCE`
/// result set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationReport {
    /// The human- and agent-readable rendered text: one line per Finding,
    /// each suffixed with its `[source: label]` citation (or `[source: none]`
    /// when the `prov:wasDerivedFrom` binding is absent).
    pub rendered_text: String,
    /// The individual answered rows with their citations.
    pub cited_answers: Vec<CitedAnswer>,
    /// The citation metrics (resolution-rate + `fabricated_count`).
    pub metrics: CitationMetrics,
}

/// Render `prov:wasDerivedFrom` source citations beside the `FINDING_PROVENANCE`
/// canned-query rows.
///
/// `rows` must be the [`QueryResult`] of the `FINDING_PROVENANCE` canned query
/// (columns: `?label ?source ?section ?assurance ?conf`). `graph` is the loaded
/// PKG (used to verify citation resolution against the live dictionary).
///
/// The function never fabricates a citation: it reads the `?source` binding
/// directly from the data rows and verifies each IRI against the graph
/// dictionary. A missing `?source` binding renders as `[source: none]`; an IRI
/// absent from the dictionary increments `fabricated_count`.
///
/// # Column layout of `FINDING_PROVENANCE`
///
/// | index | variable    | type             |
/// |-------|-------------|------------------|
/// | 0     | `?label`    | `Literal`        |
/// | 1     | `?source`   | `NamedNode` (IRI)|
/// | 2     | `?section`  | `Literal` (OPT)  |
/// | 3     | `?assurance`| `NamedNode`      |
/// | 4     | `?conf`     | `Literal`        |
pub fn render_citations(graph: &Graph, rows: &QueryResult) -> CitationReport {
    let mut cited_answers: Vec<CitedAnswer> = Vec::with_capacity(rows.rows.len());
    let mut total_citations: usize = 0;
    let mut fabricated_count: usize = 0;

    for row in &rows.rows {
        // col 0: ?label
        let finding_label = match row.first() {
            Some(Some(Term::Literal(l))) => l.value().to_string(),
            _ => String::from("(unlabelled)"),
        };

        // col 1: ?source — the prov:wasDerivedFrom IRI
        let citation = match row.get(1) {
            Some(Some(Term::NamedNode(n))) => {
                let iri = n.as_str().to_string();
                let label = local_name(&iri).to_string();
                let resolves = iri_in_dictionary(graph, &iri);
                total_citations += 1;
                if !resolves {
                    fabricated_count += 1;
                }
                Some(Citation {
                    source_iri: iri,
                    label,
                    resolves,
                })
            }
            _ => None,
        };

        // col 2: ?section (OPTIONAL)
        let section = match row.get(2) {
            Some(Some(Term::Literal(l))) => Some(l.value().to_string()),
            _ => None,
        };

        cited_answers.push(CitedAnswer {
            finding_label,
            citation,
            section,
        });
    }

    let rendered_text = render_text(&cited_answers);

    CitationReport {
        rendered_text,
        cited_answers,
        metrics: CitationMetrics {
            total_citations,
            fabricated_count,
        },
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// Build the human-/agent-readable text: one line per Finding, each suffixed
/// with its `[source: <label>]` citation and (if present) section anchor.
fn render_text(answers: &[CitedAnswer]) -> String {
    let mut out = String::new();
    for a in answers {
        out.push_str(&a.finding_label);
        match &a.citation {
            Some(c) => {
                out.push_str(&format!(" [source: {}]", c.label));
                if let Some(sec) = &a.section {
                    out.push_str(&format!(" ({})", sec));
                }
            }
            None => {
                out.push_str(" [source: none]");
            }
        }
        out.push('\n');
    }
    out
}

/// Return the local name of an IRI: the suffix after the last `#` or `/`.
/// Falls back to the full IRI when neither delimiter is present.
fn local_name(iri: &str) -> &str {
    iri.rfind(['#', '/'])
        .map(|i| &iri[i + 1..])
        .unwrap_or(iri)
}

/// Is `iri` a term in the live graph dictionary?  An IRI absent from the
/// dictionary matched no triple in the loaded graph — the citation is
/// fabricated.  Mirrors the same check in `nl_tool::iri_in_dictionary`.
fn iri_in_dictionary(graph: &Graph, iri: &str) -> bool {
    match oxrdf::NamedNode::new(iri) {
        Ok(node) => graph.id_of(&Term::NamedNode(node)).is_some(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;
    use sparq_engine::QueryResult;

    // -----------------------------------------------------------------------
    // Fixture: a tiny PKG-shaped graph with known provenance.
    //
    // Two Findings:
    //   - kb:f-with-prov  has prov:wasDerivedFrom kb:src-real
    //   - kb:f-no-prov    has NO prov:wasDerivedFrom (OPTIONAL returns nothing)
    //
    // The fixture matches the FINDING_PROVENANCE column layout so the tests
    // exercise the REAL render path, not a mock.
    // -----------------------------------------------------------------------

    /// Build the fixture graph.
    fn fixture_graph() -> Graph {
        let ttl = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .
@prefix kb:      <https://sparq.dev/ns/pkg/kb#> .

kb:src-real a pkg:Source ;
    rdfs:label "The real source"@en-GB ;
    pkg:confidence 0.9 .

kb:f-with-prov a pkg:Finding ;
    rdfs:label "Finding with provenance"@en-GB ;
    prov:wasDerivedFrom kb:src-real ;
    dcterms:source "AGENTS.md#section-anchor" ;
    pkg:assurance secx:Claimed ;
    pkg:confidence 0.85 .

kb:f-no-prov a pkg:Finding ;
    rdfs:label "Finding with no provenance"@en-GB ;
    pkg:assurance secx:Conjectured ;
    pkg:confidence 0.5 .
"#;
        Graph::load_str(ttl, "turtle").expect("fixture graph parses")
    }

    /// Run the FINDING_PROVENANCE canned query over the fixture graph and
    /// return the result.
    fn run_provenance_query(graph: &Graph) -> QueryResult {
        // Use the same SPARQL as the canned template (col layout must match).
        let sparql = r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX prov:    <http://www.w3.org/ns/prov#>
PREFIX dcterms: <http://purl.org/dc/terms/>
PREFIX rdfs:    <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?label ?source ?section ?assurance ?conf WHERE {
  ?f a pkg:Finding ;
     rdfs:label ?label ;
     prov:wasDerivedFrom ?source ;
     pkg:assurance ?assurance ;
     pkg:confidence ?conf .
  OPTIONAL { ?f dcterms:source ?section }
} ORDER BY DESC(?conf)
"#;
        sparq_engine::query(graph, sparql).expect("FINDING_PROVENANCE query runs on fixture")
    }

    // -----------------------------------------------------------------------
    // Positive test: a Finding WITH provenance renders the correct [source]
    // citation, and the metric harness asserts resolution-rate 1.0 +
    // fabricated-count 0.
    // -----------------------------------------------------------------------
    #[test]
    fn positive_finding_with_prov_renders_correct_citation_and_metrics_are_sound() {
        let graph = fixture_graph();
        let rows = run_provenance_query(&graph);

        // The fixture only has ONE Finding with prov:wasDerivedFrom (kb:f-with-prov).
        // kb:f-no-prov has none and is excluded by the mandatory join.
        assert_eq!(
            rows.rows.len(),
            1,
            "only the Finding WITH prov:wasDerivedFrom must appear; got {} rows",
            rows.rows.len()
        );

        let report = render_citations(&graph, &rows);

        // (1) The citation renders the correct source label.
        let cited = &report.cited_answers[0];
        assert!(
            cited.finding_label.contains("Finding with provenance"),
            "finding label mismatch: {}",
            cited.finding_label
        );
        let cit = cited
            .citation
            .as_ref()
            .expect("citation must be present for a Finding with prov:wasDerivedFrom");
        assert_eq!(
            cit.source_iri,
            "https://sparq.dev/ns/pkg/kb#src-real",
            "source IRI must match the prov:wasDerivedFrom triple"
        );
        assert_eq!(
            cit.label, "src-real",
            "label must be the local name of the source IRI"
        );
        assert!(
            cit.resolves,
            "src-real is present in the fixture graph — citation must resolve"
        );

        // (2) The section anchor is present.
        assert_eq!(
            cited.section.as_deref(),
            Some("AGENTS.md#section-anchor"),
            "section anchor must be the dcterms:source literal"
        );

        // (3) The rendered text contains both the label and the [source] tag.
        assert!(
            report.rendered_text.contains("Finding with provenance"),
            "rendered text missing finding label"
        );
        assert!(
            report.rendered_text.contains("[source: src-real]"),
            "rendered text missing [source] citation: {}",
            report.rendered_text
        );
        assert!(
            report.rendered_text.contains("AGENTS.md#section-anchor"),
            "rendered text missing section anchor"
        );

        // (4) Metric harness: resolution-rate 1.0, fabricated-count 0.
        assert_eq!(
            report.metrics.fabricated_count,
            0,
            "no fabricated citations expected on the fixture"
        );
        assert_eq!(
            report.metrics.total_citations,
            1,
            "one citation expected"
        );
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(
                report.metrics.citation_resolution_rate(),
                1.0,
                "citation-resolution-rate must be 1.0"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Negative test: a Finding with NO provenance must NOT render a fabricated
    // [source] citation.  The FINDING_PROVENANCE mandatory join excludes such
    // Findings from the result set (prov:wasDerivedFrom is required), so the
    // renderer sees 0 rows for them — i.e. no dangling/placeholder citation is
    // produced for kb:f-no-prov.
    // -----------------------------------------------------------------------
    #[test]
    fn negative_finding_without_prov_produces_no_fabricated_citation() {
        let graph = fixture_graph();
        let rows = run_provenance_query(&graph);

        // The mandatory prov:wasDerivedFrom join excludes kb:f-no-prov.
        assert!(
            !rows.rows.iter().any(|r| {
                matches!(&r[0], Some(Term::Literal(l)) if l.value().contains("no provenance"))
            }),
            "a Finding with no prov:wasDerivedFrom must not appear in the result set"
        );

        let report = render_citations(&graph, &rows);

        // No row for the no-provenance Finding means the renderer cannot have
        // emitted a citation for it — prove that the rendered text does NOT
        // contain a citation for that finding.
        assert!(
            !report.rendered_text.contains("no provenance"),
            "the Finding with no provenance must not appear in the rendered output: {}",
            report.rendered_text
        );

        // The metric harness still measures 0 fabricated citations.
        assert_eq!(
            report.metrics.fabricated_count,
            0,
            "fabricated-count must be 0 — no dangling citation was produced"
        );
    }

    // -----------------------------------------------------------------------
    // Metric-harness test: fabricated-citation detector fires when a row
    // carries an IRI that is NOT in the graph dictionary.  This test
    // verifies the harness is non-vacuous: it must go RED when a fabricated
    // citation is injected.
    // -----------------------------------------------------------------------
    #[test]
    fn metric_harness_detects_fabricated_citation() {
        use sparq_engine::QueryResult;

        let graph = fixture_graph();

        // Construct a synthetic result row whose ?source column contains an
        // IRI absent from the fixture graph — simulating a fabricated citation.
        let fake_source =
            oxrdf::NamedNode::new("https://sparq.dev/ns/pkg/kb#non-existent-source")
                .expect("valid IRI");
        let finding_label = oxrdf::Literal::new_simple_literal("A fabricated finding");

        // Build a minimal QueryResult matching the FINDING_PROVENANCE layout.
        let mut result = QueryResult {
            vars: vec![
                oxrdf::Variable::new("label").unwrap(),
                oxrdf::Variable::new("source").unwrap(),
                oxrdf::Variable::new("section").unwrap(),
                oxrdf::Variable::new("assurance").unwrap(),
                oxrdf::Variable::new("conf").unwrap(),
            ],
            rows: vec![],
        };
        result.rows.push(vec![
            Some(Term::Literal(finding_label)),
            Some(Term::NamedNode(fake_source)),
            None,
            None,
            None,
        ]);

        let report = render_citations(&graph, &result);

        // The detector must fire: one citation, one fabricated.
        assert_eq!(report.metrics.total_citations, 1);
        assert_eq!(
            report.metrics.fabricated_count,
            1,
            "the harness must catch a citation whose source IRI is not in the graph"
        );
        assert!(
            report.metrics.citation_resolution_rate() < 1.0,
            "resolution-rate must be < 1.0 when a fabricated citation is present"
        );

        // The citation's `resolves` flag is false.
        let cit = report.cited_answers[0]
            .citation
            .as_ref()
            .expect("citation present");
        assert!(
            !cit.resolves,
            "resolves must be false for a fabricated source IRI"
        );
    }

    // -----------------------------------------------------------------------
    // Zero-row test: an empty result set (all Findings have no provenance or
    // the graph is empty) must produce resolution-rate 1.0 and fabricated 0.
    // -----------------------------------------------------------------------
    #[test]
    fn zero_rows_gives_resolution_rate_one_and_zero_fabricated() {
        let graph = fixture_graph();
        let empty = QueryResult {
            vars: vec![],
            rows: vec![],
        };
        let report = render_citations(&graph, &empty);
        assert_eq!(report.metrics.total_citations, 0);
        assert_eq!(report.metrics.fabricated_count, 0);
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(
                report.metrics.citation_resolution_rate(),
                1.0,
                "empty result set → resolution-rate 1.0 (no fabrications)"
            );
        }
    }

    // -----------------------------------------------------------------------
    // local_name helper
    // -----------------------------------------------------------------------
    #[test]
    fn local_name_extracts_correctly() {
        assert_eq!(
            local_name("https://sparq.dev/ns/pkg/kb#src-real"),
            "src-real"
        );
        assert_eq!(local_name("https://example.org/path/to/resource"), "resource");
        assert_eq!(local_name("urn:no-delimiter"), "urn:no-delimiter");
    }

    // -----------------------------------------------------------------------
    // citation_resolution_rate edge cases
    // -----------------------------------------------------------------------
    #[test]
    fn resolution_rate_edge_cases() {
        let all_good = CitationMetrics {
            total_citations: 5,
            fabricated_count: 0,
        };
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(all_good.citation_resolution_rate(), 1.0);
        }

        let half_bad = CitationMetrics {
            total_citations: 4,
            fabricated_count: 2,
        };
        assert!((half_bad.citation_resolution_rate() - 0.5).abs() < f64::EPSILON);

        let zero = CitationMetrics {
            total_citations: 0,
            fabricated_count: 0,
        };
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(zero.citation_resolution_rate(), 1.0);
        }
    }
}
