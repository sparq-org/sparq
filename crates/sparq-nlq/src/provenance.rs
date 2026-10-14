//! The **cross-cutting provenance-join primitive** (GenAI-KB Phase 1, `sq-2489d.1`).
//!
//! This is the result-row → supporting-triples → `prov:wasDerivedFrom` /
//! `pkg:confidence` / `pkg:assurance` join that the design record
//! (`research/provenance-driven-genai-kb.md` §2 "Cross-cutting primitive" / §5 Phase 1,
//! epic `sq-2489d`) calls out as the *build-once* dependency under both Use 2
//! (answer-qualification) and Use 3 (citations). The renderer in [`crate::cite`] is a
//! thin layer on top of this.
//!
//! ## What it does
//!
//! Given a graph and a **subject IRI** (typically a binding from an answer row), it
//! pulls that subject's provenance edges:
//!
//! - `prov:wasDerivedFrom` (and its sub-property `pkg:discoveredFrom`) → source IRIs;
//! - `dcterms:source` → a section / anchor literal or IRI;
//! - `pkg:confidence` → a numeric confidence literal;
//! - `pkg:assurance` → an epistemic-basis IRI (`secx:Proven` / `Claimed` / `Conjectured`);
//! - `cito:citesAsEvidence` → a cited-evidence IRI.
//!
//! ## The load-bearing honesty invariant
//!
//! Provenance is **read out of the graph, never invented**. A subject with no provenance
//! edge produces a [`ProvenanceRecord`] for which [`ProvenanceRecord::has_provenance`] is
//! `false` — the caller renders **"no source recorded"**, never a guessed citation. This
//! is the fail-closed property the Phase-1 acceptance bar requires (over a no-provenance
//! graph the output is "no source recorded", never a guess).
//!
//! ## Generality
//!
//! The join is expressed in SPARQL over the engine, so it works for **any** graph that
//! carries PROV-O annotations — the PKG is the lowest-friction case (its Findings already
//! assert `prov:wasDerivedFrom` + `pkg:confidence` + `pkg:assurance`), but a plain
//! `prov:wasDerivedFrom`-annotated dataset resolves too. No PKG-specific assumption is
//! baked into the lookup beyond the (optional) `pkg:` predicates.

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::{query_with_budget, QueryBudget, QueryResult};

/// `prov:wasDerivedFrom` — the PROV-O floor everything attaches to.
pub const PROV_WAS_DERIVED_FROM: &str = "http://www.w3.org/ns/prov#wasDerivedFrom";
/// `dcterms:source` — the section / anchor of a Finding's source.
pub const DCTERMS_SOURCE: &str = "http://purl.org/dc/terms/source";
/// `pkg:confidence` — NET-NEW numeric confidence (0..1) on a Finding or Source.
pub const PKG_CONFIDENCE: &str = "https://sparq.dev/ns/pkg#confidence";
/// `pkg:assurance` — epistemic basis (`secx:Proven` / `Claimed` / `Conjectured`).
pub const PKG_ASSURANCE: &str = "https://sparq.dev/ns/pkg#assurance";
/// `cito:citesAsEvidence` — a typed citation edge to cited evidence.
pub const CITO_CITES_AS_EVIDENCE: &str = "http://purl.org/spar/cito/citesAsEvidence";

/// One resolved source link for a subject: the `prov:wasDerivedFrom` source IRI and,
/// when the subject also carries a `dcterms:source`, the section / anchor that locates
/// the claim within that source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLink {
    /// The source the subject was `prov:wasDerivedFrom` (or `pkg:discoveredFrom`).
    pub source: NamedNode,
    /// The `dcterms:source` section / anchor, rendered as a string (an IRI's text or a
    /// literal's lexical form). `None` when the subject has no anchor.
    pub anchor: Option<String>,
}

/// The provenance of one subject IRI: the source links plus the quality axes. This is
/// the structured citation object the [`crate::cite`] renderer turns into a footnote.
///
/// **Honesty contract.** Every field here is *read from the graph*. An empty `sources`
/// (`has_provenance() == false`) means the subject has no recorded provenance — the
/// caller must emit "no source recorded", not a fabricated reference.
#[derive(Debug, Clone, PartialEq)]
pub struct ProvenanceRecord {
    /// The subject this provenance is about (an answer-row binding).
    pub subject: NamedNode,
    /// Sources the subject was derived from, each with its optional section anchor. May
    /// be empty (the fail-closed "no source recorded" case).
    pub sources: Vec<SourceLink>,
    /// `pkg:confidence` literal lexical form, when asserted.
    pub confidence: Option<String>,
    /// `pkg:assurance` IRI text (e.g. `…#Proven`), when asserted.
    pub assurance: Option<String>,
    /// `cito:citesAsEvidence` IRIs, when asserted (a typed evidence citation, distinct
    /// from the `prov:wasDerivedFrom` lineage edge).
    pub cites_as_evidence: Vec<NamedNode>,
}

impl ProvenanceRecord {
    /// `true` iff the subject carries at least one `prov:wasDerivedFrom`/`dcterms:source`
    /// source link — i.e. a citation can be rendered for it. When `false` the honest
    /// output is "no source recorded".
    pub fn has_provenance(&self) -> bool {
        !self.sources.is_empty()
    }
}

/// IRI-safety guard: an IRI is embeddable in a `<…>` SPARQL term only if it has no `>`,
/// no whitespace, and no ASCII control characters. Bindings come from the dictionary so
/// this should always hold; we still reject defensively rather than build a malformed
/// (or injection-bearing) query.
fn iri_is_embeddable(iri: &str) -> bool {
    !iri.is_empty()
        && iri.chars().all(|c| {
            c != '>'
                && c != '<'
                && c != '"'
                && c != '{'
                && c != '}'
                && c != '|'
                && c != '\\'
                && c != '^'
                && c != '`'
                && !c.is_whitespace()
                && !c.is_control()
        })
}

/// Renders the provenance-lookup query for one subject IRI. Public + deterministic so a
/// caller can inspect exactly what is executed (mirrors the `prompt_for` discipline).
/// Returns `None` when the IRI is not embeddable in a `<…>` SPARQL term (contains `>`,
/// whitespace, or a control character — see the `iri_is_embeddable` guard).
///
/// The query is a single OPTIONAL-guarded BGP, so every axis is independently optional:
/// a subject with only `prov:wasDerivedFrom` and no `pkg:confidence` still resolves.
pub fn lookup_query(subject: &str) -> Option<String> {
    if !iri_is_embeddable(subject) {
        return None;
    }
    Some(format!(
        "PREFIX prov: <http://www.w3.org/ns/prov#>\n\
         PREFIX dcterms: <http://purl.org/dc/terms/>\n\
         PREFIX pkg: <https://sparq.dev/ns/pkg#>\n\
         PREFIX cito: <http://purl.org/spar/cito/>\n\
         SELECT ?source ?anchor ?conf ?assurance ?evidence WHERE {{\n\
         \u{20}\u{20}OPTIONAL {{ <{}> prov:wasDerivedFrom ?source }}\n\
         \u{20}\u{20}OPTIONAL {{ <{}> dcterms:source ?anchor }}\n\
         \u{20}\u{20}OPTIONAL {{ <{}> pkg:confidence ?conf }}\n\
         \u{20}\u{20}OPTIONAL {{ <{}> pkg:assurance ?assurance }}\n\
         \u{20}\u{20}OPTIONAL {{ <{}> cito:citesAsEvidence ?evidence }}\n\
         }}",
        subject, subject, subject, subject, subject
    ))
}

/// Stringifies a result-cell term for an anchor / literal value: an IRI yields its text,
/// a literal yields its lexical form, a blank node yields its `_:id`.
fn term_text(t: &Term) -> String {
    match t {
        Term::NamedNode(n) => n.as_str().to_string(),
        Term::Literal(l) => l.value().to_string(),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        // RDF-star quoted triple (the workspace enables oxrdf's `rdf-12`): render its
        // canonical N-Triples form rather than panic.
        Term::Triple(tr) => tr.to_string(),
    }
}

/// The cross-cutting primitive for ONE subject: look up its provenance edges in `graph`.
///
/// Returns a [`ProvenanceRecord`] whose `sources`/`confidence`/`assurance`/
/// `cites_as_evidence` reflect exactly what the graph asserts — possibly empty
/// (`has_provenance() == false`), the fail-closed "no source recorded" case. Read-only.
/// Runs under a bounded [`QueryBudget`] (the lookup is a tiny single-subject BGP, but the
/// budget keeps an adversarial graph from making it unbounded).
pub fn provenance_for(graph: &Graph, subject: &NamedNode) -> ProvenanceRecord {
    let empty = || ProvenanceRecord {
        subject: subject.clone(),
        sources: Vec::new(),
        confidence: None,
        assurance: None,
        cites_as_evidence: Vec::new(),
    };
    let Some(sparql) = lookup_query(subject.as_str()) else {
        return empty();
    };
    let mut budget = QueryBudget::unlimited();
    budget.max_rows = Some(100_000);
    let result = match query_with_budget(graph, &sparql, &budget) {
        Ok(r) => r,
        // A lookup failure is treated as "no provenance recorded" — fail closed, never
        // fabricate. (The query is engine-internal and fixed-shape, so this is unreachable
        // in practice, but honesty beats an unwrap.)
        Err(_) => return empty(),
    };
    records_from(subject, &result)
}

/// Index of each SELECT variable in the [`lookup_query`] result, by name.
fn col(result: &QueryResult, name: &str) -> Option<usize> {
    result.vars.iter().position(|v| v.as_str() == name)
}

/// Folds a [`lookup_query`] result table into one [`ProvenanceRecord`]. Each row is one
/// `(source, anchor, conf, assurance, evidence)` combination produced by the cross of the
/// OPTIONAL clauses; we dedupe `(source, anchor)` pairs into [`SourceLink`]s and take the
/// (single-valued) confidence/assurance from the first row that binds them.
fn records_from(subject: &NamedNode, result: &QueryResult) -> ProvenanceRecord {
    let i_source = col(result, "source");
    let i_anchor = col(result, "anchor");
    let i_conf = col(result, "conf");
    let i_assurance = col(result, "assurance");
    let i_evidence = col(result, "evidence");

    let mut sources: Vec<SourceLink> = Vec::new();
    let mut confidence: Option<String> = None;
    let mut assurance: Option<String> = None;
    let mut cites: Vec<NamedNode> = Vec::new();

    let cell = |row: &[Option<Term>], idx: Option<usize>| -> Option<Term> {
        idx.and_then(|i| row.get(i)).and_then(|c| c.clone())
    };

    for row in &result.rows {
        if let Some(Term::NamedNode(src)) = cell(row, i_source) {
            let anchor = cell(row, i_anchor).map(|t| term_text(&t));
            let link = SourceLink {
                source: src,
                anchor,
            };
            if !sources.contains(&link) {
                sources.push(link);
            }
        }
        if confidence.is_none() {
            if let Some(t) = cell(row, i_conf) {
                confidence = Some(term_text(&t));
            }
        }
        if assurance.is_none() {
            if let Some(Term::NamedNode(a)) = cell(row, i_assurance) {
                assurance = Some(a.as_str().to_string());
            }
        }
        if let Some(Term::NamedNode(e)) = cell(row, i_evidence) {
            if !cites.contains(&e) {
                cites.push(e);
            }
        }
    }

    ProvenanceRecord {
        subject: subject.clone(),
        sources,
        confidence,
        assurance,
        cites_as_evidence: cites,
    }
}

/// The **result-table join**: every distinct subject IRI bound anywhere in `result` is
/// resolved to its [`ProvenanceRecord`]. This is the entry point [`crate::cite`] uses to
/// turn an [`crate::Answer`]'s rows into citations.
///
/// "Subject IRI" here means any `Term::NamedNode` appearing in a result cell — the answer
/// bindings ARE the subjects whose provenance we cite (the design's "provenance of the
/// binding" guarantee). Order is the first-seen order of the IRIs across rows then
/// columns, so the output is deterministic. Subjects with no provenance are still
/// returned (with `has_provenance() == false`) so the caller can emit "no source
/// recorded" for them rather than silently dropping them.
pub fn join(graph: &Graph, result: &QueryResult) -> Vec<ProvenanceRecord> {
    let mut seen: Vec<NamedNode> = Vec::new();
    for row in &result.rows {
        for cell in row {
            if let Some(Term::NamedNode(n)) = cell {
                if !seen.contains(n) {
                    seen.push(n.clone());
                }
            }
        }
    }
    seen.iter().map(|s| provenance_for(graph, s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small PKG-shaped fixture: two Findings with full provenance + one bare resource
    // with none. Mirrors the real `pkg.ttl` predicate set so the join exercises the REAL
    // path, not a mock.
    const PKG: &str = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix cito:    <http://purl.org/spar/cito/> .
@prefix secx:    <https://sparq.dev/ns/secx#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:      <http://ex/> .

ex:finding1 a pkg:Finding ;
  rdfs:label "ZK soundness is internally re-audited, external sign-off pending" ;
  prov:wasDerivedFrom ex:source-threat-model ;
  dcterms:source "research/threat-model.md#zk" ;
  pkg:assurance secx:Claimed ;
  pkg:confidence "0.7" ;
  cito:citesAsEvidence ex:paper-iswc25 .

ex:finding2 a pkg:Finding ;
  rdfs:label "MPC is semi-honest only" ;
  prov:wasDerivedFrom ex:source-mpc-skill ;
  pkg:assurance secx:Proven ;
  pkg:confidence "0.9" .

ex:bare a pkg:Finding ;
  rdfs:label "a finding with no recorded provenance" .
"#;

    fn graph() -> Graph {
        Graph::load_str(PKG, "turtle").unwrap()
    }

    fn nn(iri: &str) -> NamedNode {
        NamedNode::new(iri).unwrap()
    }

    #[test]
    fn lookup_query_rejects_unembeddable_iris() {
        assert!(lookup_query("http://ex/ok").is_some());
        assert!(lookup_query("http://ex/has>angle").is_none());
        assert!(lookup_query("http://ex/has space").is_none());
        assert!(lookup_query("").is_none());
    }

    #[test]
    fn provenance_for_finding_with_full_provenance() {
        let g = graph();
        let r = provenance_for(&g, &nn("http://ex/finding1"));
        assert!(r.has_provenance());
        assert_eq!(r.sources.len(), 1);
        assert_eq!(r.sources[0].source, nn("http://ex/source-threat-model"));
        assert_eq!(
            r.sources[0].anchor.as_deref(),
            Some("research/threat-model.md#zk")
        );
        assert_eq!(r.confidence.as_deref(), Some("0.7"));
        assert_eq!(
            r.assurance.as_deref(),
            Some("https://sparq.dev/ns/secx#Claimed")
        );
        assert_eq!(r.cites_as_evidence, vec![nn("http://ex/paper-iswc25")]);
    }

    #[test]
    fn provenance_for_finding_without_anchor() {
        let g = graph();
        let r = provenance_for(&g, &nn("http://ex/finding2"));
        assert!(r.has_provenance());
        assert_eq!(r.sources.len(), 1);
        assert_eq!(r.sources[0].source, nn("http://ex/source-mpc-skill"));
        assert_eq!(r.sources[0].anchor, None);
        assert_eq!(r.confidence.as_deref(), Some("0.9"));
        assert!(r.cites_as_evidence.is_empty());
    }

    #[test]
    fn fail_closed_no_provenance_recorded() {
        // The load-bearing invariant: a subject with no provenance is NOT fabricated.
        let g = graph();
        let r = provenance_for(&g, &nn("http://ex/bare"));
        assert!(!r.has_provenance());
        assert!(r.sources.is_empty());
        assert!(r.confidence.is_none());
        assert!(r.assurance.is_none());
    }

    #[test]
    fn provenance_for_absent_subject_is_empty() {
        let g = graph();
        let r = provenance_for(&g, &nn("http://ex/does-not-exist"));
        assert!(!r.has_provenance());
    }

    /// A subject with TWO `prov:wasDerivedFrom` sources resolves to TWO distinct
    /// [`SourceLink`]s — the multi-source fold (`records_from` dedup of the OPTIONAL
    /// cross-product into one link per distinct `(source, anchor)`). No existing fixture
    /// has a subject with more than one source.
    #[test]
    fn provenance_for_subject_with_two_sources() {
        let g = Graph::load_str(
            r#"@prefix prov: <http://www.w3.org/ns/prov#> .
               @prefix pkg:  <https://sparq.dev/ns/pkg#> .
               @prefix ex:   <http://ex/> .
               ex:m a pkg:Finding ;
                 prov:wasDerivedFrom ex:s1 , ex:s2 ;
                 pkg:assurance <https://sparq.dev/ns/secx#Claimed> ;
                 pkg:confidence "0.6" ."#,
            "turtle",
        )
        .unwrap();
        let r = provenance_for(&g, &nn("http://ex/m"));
        assert!(r.has_provenance());
        // Both sources present, de-duplicated and order-independent.
        let mut sources: Vec<&str> = r.sources.iter().map(|l| l.source.as_str()).collect();
        sources.sort_unstable();
        assert_eq!(sources, vec!["http://ex/s1", "http://ex/s2"]);
        // The single-valued quality axes are read once, not duplicated per source row.
        assert_eq!(r.confidence.as_deref(), Some("0.6"));
        assert_eq!(
            r.assurance.as_deref(),
            Some("https://sparq.dev/ns/secx#Claimed")
        );
    }

    /// `lookup_query` is deterministic and embeds the subject in EVERY axis clause — the
    /// shape a caller inspecting "exactly what is executed" depends on.
    #[test]
    fn lookup_query_embeds_subject_in_every_axis() {
        let q = lookup_query("http://ex/s").expect("embeddable");
        // The subject appears once per OPTIONAL axis (5 axes).
        assert_eq!(q.matches("<http://ex/s>").count(), 5);
        // All five provenance predicates are present.
        assert!(q.contains("prov:wasDerivedFrom"));
        assert!(q.contains("dcterms:source"));
        assert!(q.contains("pkg:confidence"));
        assert!(q.contains("pkg:assurance"));
        assert!(q.contains("cito:citesAsEvidence"));
        // Determinism: same input → byte-identical query.
        assert_eq!(lookup_query("http://ex/s").unwrap(), q);
    }

    #[test]
    fn join_resolves_every_bound_iri() {
        let g = graph();
        // Simulate an answer table binding the three subjects.
        let result = query_with_budget(
            &g,
            "SELECT ?f WHERE { ?f a <https://sparq.dev/ns/pkg#Finding> }",
            &QueryBudget::unlimited(),
        )
        .unwrap();
        let records = join(&g, &result);
        // Three findings -> three records; two have provenance, one does not.
        assert_eq!(records.len(), 3);
        let with = records.iter().filter(|r| r.has_provenance()).count();
        let without = records.iter().filter(|r| !r.has_provenance()).count();
        assert_eq!(with, 2);
        assert_eq!(without, 1);
    }
}
