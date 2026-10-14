//! The **PKG-native citation renderer** (GenAI-KB Phase 1, `sq-2489d.1`, Use 3).
//!
//! Turns the provenance resolved by [`crate::provenance`] into **cited footnotes**: each
//! answer-row binding becomes a numbered [`Citation`] (`[1]` → source IRI + section
//! anchor + the quality axes that justify it). This is the lowest-risk, highest-value
//! slice of `research/provenance-driven-genai-kb.md` §2 Use 3 / §5 Phase 1 — only the
//! renderer was missing; the provenance was already queryable.
//!
//! ## The honesty win (citations are emitted, never generated)
//!
//! Citations are produced **from in-graph provenance**, not by a language model. A
//! citation can therefore only point at a `prov:wasDerivedFrom` source that the graph
//! actually asserts — so the **citation-resolution rate is 1.0 by construction** (every
//! emitted citation's `source` is a real in-graph IRI) and there are **zero fabricated
//! citations**. This is the structural guarantee the design record contrasts with
//! text-RAG, where a citation is a post-hoc retrieval guess.
//!
//! Over a graph with **no** provenance the renderer emits no citation for that subject
//! and records it in [`CitedAnswer::no_source_recorded`] — the honest "no source
//! recorded", **never a guess** (the Phase-1 acceptance bar).

use oxrdf::NamedNode;
use sparq_core::Graph;
use sparq_engine::QueryResult;

use crate::provenance::{self, ProvenanceRecord};

/// One rendered citation: a numbered reference resolving an answer-row binding to a real
/// in-graph source. The `marker` is an ALCE-style inline marker (`[1]`) so a downstream
/// LLM verbalisation can carry the reference back to the IRI.
#[derive(Debug, Clone, PartialEq)]
pub struct Citation {
    /// 1-based citation number (the `[n]` marker index).
    pub number: usize,
    /// The subject this citation is for (the answer-row binding being cited).
    pub subject: NamedNode,
    /// The `prov:wasDerivedFrom` source IRI. Always a **real in-graph IRI** — the
    /// resolution guarantee.
    pub source: NamedNode,
    /// The `dcterms:source` section / anchor within the source, when present.
    pub anchor: Option<String>,
    /// The `pkg:confidence` lexical value, when asserted.
    pub confidence: Option<String>,
    /// The `pkg:assurance` IRI text (`…#Proven` / `Claimed` / `Conjectured`), when
    /// asserted.
    pub assurance: Option<String>,
}

impl Citation {
    /// The ALCE-style inline marker, e.g. `[1]`.
    pub fn marker(&self) -> String {
        format!("[{}]", self.number)
    }

    /// A one-line footnote string: `[n] <source>#anchor (assurance, conf=…)`. Built only
    /// from asserted provenance — no field is invented.
    pub fn footnote(&self) -> String {
        let mut s = format!("{} <{}>", self.marker(), self.source.as_str());
        if let Some(anchor) = &self.anchor {
            s.push_str(&format!(" — {}", anchor));
        }
        let mut quals: Vec<String> = Vec::new();
        if let Some(a) = &self.assurance {
            // Show just the local name of the assurance IRI for readability.
            let local = a.rsplit(['#', '/']).next().unwrap_or(a.as_str());
            quals.push(local.to_string());
        }
        if let Some(c) = &self.confidence {
            quals.push(format!("conf={}", c));
        }
        if !quals.is_empty() {
            s.push_str(&format!(" ({})", quals.join(", ")));
        }
        s
    }
}

/// The citations rendered for an answer, plus the subjects for which **no source was
/// recorded** — kept explicit so the caller can honestly report them rather than silently
/// dropping them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CitedAnswer {
    /// Numbered citations, in first-seen subject order. Every entry resolves to a real
    /// in-graph source.
    pub citations: Vec<Citation>,
    /// Subjects that appeared in the result but carry no `prov:wasDerivedFrom` provenance
    /// — the "no source recorded" set. **Never** turned into a citation.
    pub no_source_recorded: Vec<NamedNode>,
}

impl CitedAnswer {
    /// `true` when at least one citation was rendered.
    pub fn has_citations(&self) -> bool {
        !self.citations.is_empty()
    }

    /// The rendered footnote block — one `[n] …` line per citation, plus a trailing
    /// "no source recorded" line listing any un-sourced subjects. Empty string when there
    /// is nothing to cite and nothing un-sourced.
    pub fn footnotes(&self) -> String {
        let mut lines: Vec<String> = self.citations.iter().map(Citation::footnote).collect();
        if !self.no_source_recorded.is_empty() {
            let subs: Vec<String> = self
                .no_source_recorded
                .iter()
                .map(|s| format!("<{}>", s.as_str()))
                .collect();
            lines.push(format!("no source recorded: {}", subs.join(", ")));
        }
        lines.join("\n")
    }

    /// Citation-resolution rate over THIS answer: the fraction of rendered citations whose
    /// source is a real in-graph IRI. Because every [`Citation`] is built from an in-graph
    /// `prov:wasDerivedFrom` edge, this is `1.0` whenever any citation was rendered and
    /// `1.0` (vacuously) when none was — the structural guarantee, computed not assumed.
    /// The `graph` argument lets a caller re-verify each source resolves, closing the loop
    /// the acceptance bar measures.
    pub fn resolution_rate(&self, graph: &Graph) -> f64 {
        if self.citations.is_empty() {
            return 1.0;
        }
        let resolved = self
            .citations
            .iter()
            .filter(|c| graph.id_of(&c.source.clone().into()).is_some())
            .count();
        resolved as f64 / self.citations.len() as f64
    }
}

/// Render citations for one [`ProvenanceRecord`] list (the output of
/// [`provenance::join`]). Each record with provenance yields exactly one [`Citation`] per
/// distinct source link (a Finding derived from two sources cites both); each record
/// **without** provenance is recorded in [`CitedAnswer::no_source_recorded`]. Numbering is
/// assigned in render order, starting at 1.
pub fn render(records: &[ProvenanceRecord]) -> CitedAnswer {
    let mut out = CitedAnswer::default();
    let mut n = 0usize;
    for rec in records {
        if rec.has_provenance() {
            for link in &rec.sources {
                n += 1;
                out.citations.push(Citation {
                    number: n,
                    subject: rec.subject.clone(),
                    source: link.source.clone(),
                    anchor: link.anchor.clone(),
                    confidence: rec.confidence.clone(),
                    assurance: rec.assurance.clone(),
                });
            }
        } else {
            out.no_source_recorded.push(rec.subject.clone());
        }
    }
    out
}

/// The end-to-end entry point: run the cross-cutting provenance [`join`](provenance::join)
/// over a [`QueryResult`] and render the result into a [`CitedAnswer`]. This is what
/// [`crate::Answer::citations`] calls. Read-only over `graph`.
pub fn cite_result(graph: &Graph, result: &QueryResult) -> CitedAnswer {
    let records = provenance::join(graph, result);
    render(&records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_engine::{query_with_budget, QueryBudget};

    const PKG: &str = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix secx:    <https://sparq.dev/ns/secx#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:      <http://ex/> .

ex:finding1 a pkg:Finding ;
  rdfs:label "ZK soundness internally re-audited, external sign-off pending" ;
  prov:wasDerivedFrom ex:source-threat-model ;
  dcterms:source "research/threat-model.md#zk" ;
  pkg:assurance secx:Claimed ;
  pkg:confidence "0.7" .

ex:finding2 a pkg:Finding ;
  rdfs:label "MPC is semi-honest only" ;
  prov:wasDerivedFrom ex:source-mpc-skill ;
  pkg:assurance secx:Proven ;
  pkg:confidence "0.9" .

ex:bare a pkg:Finding ;
  rdfs:label "no recorded provenance" .
"#;

    fn graph() -> Graph {
        Graph::load_str(PKG, "turtle").unwrap()
    }

    fn nn(iri: &str) -> NamedNode {
        NamedNode::new(iri).unwrap()
    }

    fn all_findings(g: &Graph) -> QueryResult {
        query_with_budget(
            g,
            "SELECT ?f WHERE { ?f a <https://sparq.dev/ns/pkg#Finding> } ORDER BY ?f",
            &QueryBudget::unlimited(),
        )
        .unwrap()
    }

    #[test]
    fn renders_numbered_citations_for_provenanced_subjects() {
        let g = graph();
        let cited = cite_result(&g, &all_findings(&g));
        assert!(cited.has_citations());
        // finding1 + finding2 cite, bare does not.
        assert_eq!(cited.citations.len(), 2);
        assert_eq!(cited.citations[0].number, 1);
        assert_eq!(cited.citations[1].number, 2);
        assert_eq!(cited.no_source_recorded, vec![nn("http://ex/bare")]);
    }

    #[test]
    fn citation_resolution_rate_is_one_by_construction() {
        // The Phase-1 acceptance metric: every emitted citation resolves to a real
        // in-graph source IRI.
        let g = graph();
        let cited = cite_result(&g, &all_findings(&g));
        assert_eq!(cited.resolution_rate(&g), 1.0);
        for c in &cited.citations {
            assert!(
                g.id_of(&c.source.clone().into()).is_some(),
                "citation {} points at a non-existent source",
                c.number
            );
        }
    }

    #[test]
    fn zero_fabricated_citations_over_no_provenance_graph() {
        // Over a graph that carries ZERO provenance, the renderer emits NO citation and
        // records every subject as "no source recorded" — never a guess.
        let bare = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:a ex:knows ex:b .
               ex:b ex:knows ex:c ."#,
            "turtle",
        )
        .unwrap();
        let result = query_with_budget(
            &bare,
            "SELECT ?s ?o WHERE { ?s <http://ex/knows> ?o }",
            &QueryBudget::unlimited(),
        )
        .unwrap();
        let cited = cite_result(&bare, &result);
        assert!(!cited.has_citations());
        assert_eq!(cited.citations.len(), 0);
        // Every bound subject is accounted for as un-sourced, none dropped.
        assert!(!cited.no_source_recorded.is_empty());
        // Vacuous resolution rate is 1.0 (nothing fabricated to be wrong).
        assert_eq!(cited.resolution_rate(&bare), 1.0);
    }

    #[test]
    fn footnote_only_uses_asserted_fields() {
        let g = graph();
        let cited = cite_result(&g, &all_findings(&g));
        let f1 = cited
            .citations
            .iter()
            .find(|c| c.subject == nn("http://ex/finding1"))
            .unwrap();
        let fn1 = f1.footnote();
        assert!(fn1.contains("<http://ex/source-threat-model>"));
        assert!(fn1.contains("research/threat-model.md#zk"));
        assert!(fn1.contains("Claimed"));
        assert!(fn1.contains("conf=0.7"));
        assert!(fn1.starts_with("[1]") || fn1.starts_with("[2]"));

        // finding2 has no anchor -> the footnote must not invent one.
        let f2 = cited
            .citations
            .iter()
            .find(|c| c.subject == nn("http://ex/finding2"))
            .unwrap();
        let fn2 = f2.footnote();
        assert!(fn2.contains("<http://ex/source-mpc-skill>"));
        assert!(!fn2.contains("research/"));
        assert!(fn2.contains("Proven"));
    }

    #[test]
    fn footnotes_block_lists_unsourced_subjects() {
        let g = graph();
        let cited = cite_result(&g, &all_findings(&g));
        let block = cited.footnotes();
        assert!(block.contains("no source recorded"));
        assert!(block.contains("<http://ex/bare>"));
    }

    /// A subject derived from TWO sources must cite BOTH, with the numbering continuing
    /// across the two citations (not reset per subject) — the `render` contract: "one
    /// `Citation` per distinct source link" / "a Finding derived from two sources cites
    /// both". This is the multi-source branch of the `for link in &rec.sources` loop, which
    /// no single-source fixture exercises.
    #[test]
    fn subject_with_two_sources_cites_both_with_continuing_numbers() {
        let g = Graph::load_str(
            r#"@prefix pkg:  <https://sparq.dev/ns/pkg#> .
               @prefix prov: <http://www.w3.org/ns/prov#> .
               @prefix ex:   <http://ex/> .
               ex:multi a pkg:Finding ;
                 prov:wasDerivedFrom ex:src-a , ex:src-b ;
                 pkg:confidence "0.8" .
               ex:src-a a pkg:Source .
               ex:src-b a pkg:Source ."#,
            "turtle",
        )
        .unwrap();
        // Bind ONLY the multi-source finding so the citation count is exactly its sources.
        let result = query_with_budget(
            &g,
            "PREFIX pkg: <https://sparq.dev/ns/pkg#> \
             SELECT ?f WHERE { VALUES ?f { <http://ex/multi> } ?f a pkg:Finding }",
            &QueryBudget::unlimited(),
        )
        .unwrap();
        let cited = cite_result(&g, &result);

        // Two citations, BOTH for the same subject, BOTH to a real distinct source.
        assert_eq!(cited.citations.len(), 2, "both sources must be cited");
        assert!(cited
            .citations
            .iter()
            .all(|c| c.subject == nn("http://ex/multi")));
        let mut srcs: Vec<&str> = cited.citations.iter().map(|c| c.source.as_str()).collect();
        srcs.sort_unstable();
        assert_eq!(srcs, vec!["http://ex/src-a", "http://ex/src-b"]);
        // Numbering continues 1,2 across the sources — it does NOT reset to 1 per source.
        let mut nums: Vec<usize> = cited.citations.iter().map(|c| c.number).collect();
        nums.sort_unstable();
        assert_eq!(nums, vec![1, 2]);
        // The subject's shared confidence is carried onto each per-source citation.
        assert!(cited
            .citations
            .iter()
            .all(|c| c.confidence.as_deref() == Some("0.8")));
        // The multi-source path keeps the resolution guarantee.
        assert_eq!(cited.resolution_rate(&g), 1.0);
    }

    /// Citation numbering is a stable, document-wide index across distinct subjects — the
    /// property a downstream `[n]` marker relies on. Asserts the markers render `[1]`/`[2]`
    /// and each footnote leads with its own marker.
    #[test]
    fn numbering_is_document_wide_and_marker_leads_footnote() {
        let g = graph();
        let cited = cite_result(&g, &all_findings(&g));
        assert_eq!(cited.citations.len(), 2);
        assert_eq!(cited.citations[0].marker(), "[1]");
        assert_eq!(cited.citations[1].marker(), "[2]");
        assert!(cited.citations[0].footnote().starts_with("[1] "));
        assert!(cited.citations[1].footnote().starts_with("[2] "));
    }

    /// `render` over an EMPTY record list yields a citation-free answer with an empty
    /// footnote block and a vacuous resolution rate of 1.0 — the degenerate input that
    /// `cite_result` over an empty result table produces.
    #[test]
    fn render_of_no_records_is_empty_and_vacuously_resolved() {
        let g = graph();
        let cited = render(&[]);
        assert!(!cited.has_citations());
        assert!(cited.no_source_recorded.is_empty());
        assert_eq!(cited.footnotes(), "");
        assert_eq!(cited.resolution_rate(&g), 1.0);
    }
}
