//! The **deterministic grounding-resolver** — propose-then-verify (§4.3, the OPEN
//! `sq-0wo9e.6` pattern, realised here for the fixture pipeline).
//!
//! sq-2489d.5. A cheap model **will hallucinate claims and mis-cite**. The
//! SHACL gate checks *structure*, not *truth*; the strongest deterministic anchor we can
//! add is to require, before a candidate Finding is allowed to be committed, that:
//!
//! 1. its **justification is an entailed span of the source abstract** — a normalised
//!    substring of the abstract the candidate was extracted from (so a fabricated
//!    "justification" that does not appear in the text is rejected); AND
//! 2. every **cited DOI resolves to a `pkg:Source` actually in the batch** (so a dangling
//!    citation to a paper not in the corpus is rejected).
//!
//! A candidate that fails EITHER check is **quarantined, not committed** (the
//! `super::pipeline` does the quarantining; this module is the pure verifier). Neither
//! check establishes truth — a well-formed, in-corpus, text-anchored claim can still be
//! false; the docs say so and the machine tier stays `secx:Conjectured` accordingly.

use std::collections::HashSet;

use super::connector::SourceStub;
use super::extract::CandidateFinding;

/// Why a candidate failed grounding (for the quarantine sidecar — never silently dropped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroundingFailure {
    /// The source DOI the candidate was extracted from is not in the batch (should not
    /// happen for a recorded extractor, but checked defensively).
    UnknownSource,
    /// The justification is not an entailed (normalised-substring) span of the abstract.
    JustificationNotAnchored,
    /// A cited DOI does not resolve to a `pkg:Source` in the batch (dangling citation).
    DanglingCitation(String),
}

impl GroundingFailure {
    /// A short human-readable reason for the quarantine sidecar.
    pub fn reason(&self) -> String {
        match self {
            GroundingFailure::UnknownSource => "source DOI not in batch".to_string(),
            GroundingFailure::JustificationNotAnchored => {
                "justification is not a span of the source abstract".to_string()
            }
            // POSITIONAL format args (CodeQL rust/unused-variable false-positive guard).
            GroundingFailure::DanglingCitation(doi) => {
                format!("cited DOI does not resolve to an in-batch Source: {}", doi)
            }
        }
    }
}

/// Normalise text for the substring-span check: lower-case, collapse all runs of ASCII
/// whitespace to a single space, and trim. This makes the "is the justification a span of
/// the abstract" check robust to the cosmetic whitespace/case differences a model
/// introduces while still requiring the *content* to come from the text (it cannot let a
/// fabricated claim through — the words must actually be present, in order).
pub fn normalise_span(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// Verify ONE candidate against the batch. Returns `Ok(())` if it is grounded (its
/// justification is a span of its source's abstract AND every cited DOI is an in-batch
/// Source), else the first [`GroundingFailure`]. Pure + deterministic — no I/O.
pub fn verify(
    candidate: &CandidateFinding,
    abstract_by_doi: &std::collections::HashMap<String, String>,
    source_dois: &HashSet<String>,
) -> Result<(), GroundingFailure> {
    // 1. The justification must be a normalised span of the source abstract.
    let Some(abstract_text) = abstract_by_doi.get(&candidate.source_doi) else {
        return Err(GroundingFailure::UnknownSource);
    };
    let hay = normalise_span(abstract_text);
    let needle = normalise_span(&candidate.justification);
    if needle.is_empty() || !hay.contains(&needle) {
        return Err(GroundingFailure::JustificationNotAnchored);
    }
    // 2. Every cited DOI must resolve to a Source in the batch.
    for cited in &candidate.cited_dois {
        if !source_dois.contains(cited) {
            return Err(GroundingFailure::DanglingCitation(cited.clone()));
        }
    }
    Ok(())
}

/// Build the `(abstract-by-DOI, source-DOI-set)` lookups the [`verify`] resolver needs from
/// the batch of source stubs. Convenience for the pipeline.
pub fn index(stubs: &[SourceStub]) -> (std::collections::HashMap<String, String>, HashSet<String>) {
    let mut abstracts = std::collections::HashMap::new();
    let mut dois = HashSet::new();
    for s in stubs {
        abstracts.insert(s.doi.clone(), s.abstract_text.clone());
        dois.insert(s.doi.clone());
    }
    (abstracts, dois)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::literature::connector::parse_openalex_batch;
    use crate::literature::extract::{Extractor, RecordedExtractor};

    fn batch() -> (
        Vec<SourceStub>,
        std::collections::HashMap<String, String>,
        HashSet<String>,
    ) {
        let (stubs, _) = parse_openalex_batch(crate::literature::FIXTURE_OPENALEX_BATCH).unwrap();
        let (abstracts, dois) = index(&stubs);
        (stubs, abstracts, dois)
    }

    #[test]
    fn normalise_span_collapses_whitespace_and_case() {
        assert_eq!(normalise_span("  Foo\n  BAR  baz "), "foo bar baz");
    }

    #[test]
    fn grounded_candidate_passes() {
        let (stubs, abstracts, dois) = batch();
        let cands = RecordedExtractor::from_fixture()
            .unwrap()
            .extract(&stubs)
            .unwrap();
        // The first candidate's justification IS a span of the rdf-parallel-parse abstract.
        let good = cands
            .iter()
            .find(|c| c.justification.starts_with("We present a chunk-parallel"))
            .unwrap();
        assert!(verify(good, &abstracts, &dois).is_ok());
    }

    #[test]
    fn fabricated_justification_is_quarantined() {
        let (stubs, abstracts, dois) = batch();
        let cands = RecordedExtractor::from_fixture()
            .unwrap()
            .extract(&stubs)
            .unwrap();
        // The fabricated "10x speedup ... provably optimal" claim is NOT in any abstract.
        let bad = cands
            .iter()
            .find(|c| c.justification.contains("provably optimal"))
            .unwrap();
        assert_eq!(
            verify(bad, &abstracts, &dois),
            Err(GroundingFailure::JustificationNotAnchored)
        );
    }

    #[test]
    fn dangling_citation_is_quarantined() {
        let (stubs, abstracts, dois) = batch();
        let cands = RecordedExtractor::from_fixture()
            .unwrap()
            .extract(&stubs)
            .unwrap();
        let dangling = cands
            .iter()
            .find(|c| c.cited_dois.iter().any(|d| d.contains("nonexistent")))
            .unwrap();
        assert!(matches!(
            verify(dangling, &abstracts, &dois),
            Err(GroundingFailure::DanglingCitation(_))
        ));
    }
}
