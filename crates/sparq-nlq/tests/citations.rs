//! End-to-end citation rendering over the FULL NL→SPARQL loop (GenAI-KB Phase 1,
//! `sq-2489d.1`). These exercise the REAL path — the `Nlq::ask` loop parses, executes
//! under a budget, and produces a real `Answer`; only then does `Answer::citations`
//! resolve the answer rows to in-graph provenance. No mock bypasses the join.
//!
//! The acceptance bar (research/provenance-driven-genai-kb.md §5 Phase 1): on a
//! PKG-answerable fixture set the **citation-resolution rate is 1.0** (every emitted
//! citation resolves to a real in-graph Source/anchor), there are **zero fabricated
//! citations**, and over a no-provenance graph the output is "no source recorded", never
//! a guess.
#![cfg(feature = "citations")]

use sparq_core::Graph;
use sparq_nlq::{Llm, Nlq};

/// A scripted `Llm` that returns one fixed SPARQL query regardless of the prompt — so the
/// test drives the REAL `ask` loop (validate → execute) without depending on an
/// exact-prompt fixture. The provenance join then runs on the real `Answer.result`.
struct FixedSparql(String);
impl Llm for FixedSparql {
    fn complete(&self, _prompt: &str) -> Result<String, String> {
        Ok(format!("```sparql\n{}\n```", self.0))
    }
}

/// A PKG-shaped fixture mirroring the real `crates/sparq-kb/ingest/agents-findings.ttl`
/// Finding shape: a source, a `prov:wasDerivedFrom`, a `dcterms:source` anchor, a
/// `pkg:confidence`, and a `pkg:assurance` — plus one Finding with NO provenance.
const PKG: &str = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix cito:    <http://purl.org/spar/cito/> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix kb:      <https://sparq.dev/ns/pkg/kb#> .

kb:doc-agents-md a pkg:Source ;
  dcterms:title "AGENTS.md — the sparq agent charter" ;
  pkg:exploredStatus pkg:Explored ;
  pkg:confidence 1.0 .

kb:find-merge-ci-summary-gate a pkg:Finding ;
  rdfs:label "A PR merges only when ci-summary is green and every review thread is resolved" ;
  prov:wasDerivedFrom kb:doc-agents-md ;
  cito:citesAsEvidence kb:doc-agents-md ;
  dcterms:source "AGENTS.md#contribution-workflow" ;
  pkg:confidence 0.98 ;
  pkg:assurance secx:Claimed .

kb:find-zk-member-checklist a pkg:Finding ;
  rdfs:label "Adding a zk/compose member needs the gate-count snapshot updated" ;
  prov:wasDerivedFrom kb:doc-agents-md ;
  dcterms:source "AGENTS.md#post-batch-re-evaluation-checklist" ;
  pkg:confidence 0.95 ;
  pkg:assurance secx:Proven .

kb:find-no-prov a pkg:Finding ;
  rdfs:label "a finding without recorded provenance" .
"#;

fn pkg_graph() -> Graph {
    Graph::load_str(PKG, "turtle").unwrap()
}

#[test]
fn citation_resolution_rate_is_one_over_pkg_answer() {
    let graph = pkg_graph();
    // The NL question is "which findings have provenance?" → a SELECT over the two
    // sourced findings. The loop runs for real; the answer rows are the citable subjects.
    let sparql = "PREFIX pkg: <https://sparq.dev/ns/pkg#> \
                  PREFIX prov: <http://www.w3.org/ns/prov#> \
                  SELECT ?f WHERE { ?f a pkg:Finding ; prov:wasDerivedFrom ?s } ORDER BY ?f";
    let nlq = Nlq::new(&graph, Box::new(FixedSparql(sparql.to_string())));
    let answer = nlq.ask("which findings have a recorded source?").unwrap();
    assert_eq!(answer.result.len(), 2, "two sourced findings");

    let cited = answer.citations(&graph);
    assert!(cited.has_citations());
    assert_eq!(cited.citations.len(), 2);
    // THE ACCEPTANCE METRIC: every emitted citation resolves to a real in-graph source.
    assert_eq!(cited.resolution_rate(&graph), 1.0);
    for c in &cited.citations {
        assert!(
            graph.id_of(&c.source.clone().into()).is_some(),
            "citation source must be a real in-graph IRI"
        );
    }
    // No fabricated subjects: every cited subject is a real Finding IRI bound by the query.
    for c in &cited.citations {
        assert!(c.subject.as_str().contains("/kb#find-"));
    }
}

#[test]
fn no_source_recorded_never_a_guess() {
    let graph = pkg_graph();
    // Ask over the no-provenance finding specifically.
    let sparql = "PREFIX pkg: <https://sparq.dev/ns/pkg#> \
                  PREFIX kb:  <https://sparq.dev/ns/pkg/kb#> \
                  SELECT ?f WHERE { VALUES ?f { kb:find-no-prov } ?f a pkg:Finding }";
    let nlq = Nlq::new(&graph, Box::new(FixedSparql(sparql.to_string())));
    let answer = nlq.ask("the un-sourced finding").unwrap();
    assert_eq!(answer.result.len(), 1);

    let cited = answer.citations(&graph);
    // No citation is fabricated for the un-sourced subject; it is reported as such.
    assert!(!cited.has_citations());
    assert_eq!(cited.citations.len(), 0);
    assert_eq!(cited.no_source_recorded.len(), 1);
    assert!(cited.footnotes().contains("no source recorded"));
    // Vacuous resolution is 1.0 — nothing wrong was emitted.
    assert_eq!(cited.resolution_rate(&graph), 1.0);
}

#[test]
fn zero_fabricated_citations_over_a_bare_graph() {
    // A graph with ZERO provenance vocabulary at all.
    let graph = Graph::load_str(
        r#"@prefix ex: <http://ex/> .
           ex:alice ex:knows ex:bob .
           ex:bob   ex:knows ex:carol ."#,
        "turtle",
    )
    .unwrap();
    let nlq = Nlq::new(
        &graph,
        Box::new(FixedSparql(
            "SELECT ?p ?o WHERE { ?p <http://ex/knows> ?o }".to_string(),
        )),
    );
    let answer = nlq.ask("who knows whom?").unwrap();
    assert!(!answer.result.is_empty());

    let cited = answer.citations(&graph);
    assert!(!cited.has_citations());
    assert_eq!(cited.citations.len(), 0);
    // Every bound subject is accounted for as un-sourced — none silently dropped.
    assert!(!cited.no_source_recorded.is_empty());
    assert_eq!(cited.resolution_rate(&graph), 1.0);
}
