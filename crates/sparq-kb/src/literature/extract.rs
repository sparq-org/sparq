//! The **record/replay extract trait** — the only LLM-in-the-loop step, isolated so CI
//! runs on a committed tape and makes ZERO live model calls (§4.7).
//!
//! sq-2489d.5. A cheap-model extractor turns a paper's abstract into candidate
//! `pkg:Finding`s. That is the single non-deterministic, credential-requiring step in the
//! pipeline; everything around it (connector, grounding, emit, SHACL gate) is
//! deterministic. By expressing extraction as the [`Extractor`] trait we can:
//!
//! - run CI entirely on [`RecordedExtractor`] (replays [`super::FIXTURE_EXTRACTIONS`]), and
//! - drop in a real cheap-model batch extractor later (Phase 6, `needs-access`) behind the
//!   *same* trait, with no change to the rest of the pipeline.

use serde_json::Value;
use std::collections::HashMap;

use super::connector::SourceStub;

/// One candidate Finding a (possibly machine) extractor proposes from a source's abstract.
/// This is the raw, *unverified, uncommitted* proposal — the grounding-resolver
/// (`super::ground`) decides whether it may be committed.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateFinding {
    /// The DOI of the source this candidate was extracted from (the join key back to the
    /// [`SourceStub`]).
    pub source_doi: String,
    /// The verdict the extractor assigned (`yes` / `no` / `partial` / free text).
    pub verdict: String,
    /// Confidence in `[0, 1]` (the extractor's self-reported weight).
    pub confidence: f64,
    /// The epistemic basis the extractor asserted (`Proven` / `Claimed` / `Conjectured`).
    /// NOTE: the machine tier is *capped* at `Conjectured` by the literature SHACL shapes;
    /// a candidate that asserts `Proven` will be quarantined at the SHACL gate even if it
    /// survives grounding. The pipeline ALSO downgrades it defensively (`super::pipeline`).
    pub assurance: String,
    /// The justification the extractor gave — REQUIRED by the grounding-resolver to be an
    /// entailed (substring, normalised) span of the source abstract (§4.3).
    pub justification: String,
    /// The DOIs the candidate cites as evidence — REQUIRED by the grounding-resolver to
    /// resolve to a `pkg:Source` actually in the batch.
    pub cited_dois: Vec<String>,
}

/// One candidate REJECTED at an extractor's defensive boundary (the live extractor's
/// `apply_defensive_caps` drops likely hallucinations BEFORE the pipeline sees them —
/// `sq-tx8v9`). Carries the claimed source DOI plus a stable reason category ONLY — no
/// justification text — so it is safe to thread verbatim into the committed run sidecar
/// (fail-closed licensing: no abstract-derived text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryReject {
    /// The source DOI the rejected candidate claimed.
    pub source_doi: String,
    /// The stable reason category (the same keys the pilot's quarantine metrics use:
    /// `justification-not-anchored` | `unknown-source`).
    pub reason: String,
}

/// The extraction step, abstracted so the live (model) and replay (fixture) implementations
/// are interchangeable. `extract` takes the normalised source stubs and returns the
/// candidate Findings per source. A live implementation would call a cheap-model batch
/// API here; [`RecordedExtractor`] replays a committed tape.
pub trait Extractor {
    /// Extract candidate Findings for each source stub. Errors are surfaced (never
    /// swallowed) so the pipeline can quarantine a source whose extraction failed rather
    /// than silently produce zero findings for it.
    fn extract(&self, stubs: &[SourceStub]) -> Result<Vec<CandidateFinding>, String>;

    /// The candidates this extractor REJECTED at its defensive boundary, accumulated
    /// across every `extract` call since construction. INVARIANT (`sq-tx8v9`): boundary
    /// rejects are COUNTED, never silently dropped — the pilot threads these into the run
    /// metrics so the fetched→extracted honesty gap is visible in the sidecar. The
    /// default is empty: a replay extractor applies no boundary.
    fn boundary_rejects(&self) -> Vec<BoundaryReject> {
        Vec::new()
    }
}

/// The **replay** extractor: returns the candidate Findings recorded in a committed tape,
/// keyed by source DOI. Makes NO network/model call — it is a pure lookup over the parsed
/// fixture. This is the only [`Extractor`] shipped in Phase 5.
#[derive(Debug, Clone)]
pub struct RecordedExtractor {
    by_doi: HashMap<String, Vec<CandidateFinding>>,
}

impl RecordedExtractor {
    /// Build a replay extractor from a recorded extraction tape (the
    /// [`super::FIXTURE_EXTRACTIONS`] JSON shape). Pure; no I/O beyond reading the string.
    pub fn from_tape(json: &str) -> Result<Self, String> {
        let root: Value =
            serde_json::from_str(json).map_err(|e| format!("extraction tape JSON: {}", e))?;
        let entries = root
            .get("extractions")
            .and_then(Value::as_array)
            .ok_or_else(|| "extraction tape: missing `extractions` array".to_string())?;

        let mut by_doi: HashMap<String, Vec<CandidateFinding>> = HashMap::new();
        for entry in entries {
            let source_doi = entry
                .get("source_doi")
                .and_then(Value::as_str)
                .and_then(super::connector::normalise_doi)
                .ok_or_else(|| "extraction tape: an entry has no source_doi".to_string())?;
            let candidates = entry
                .get("candidates")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    format!(
                        "extraction tape: entry {} has no candidates array",
                        source_doi
                    )
                })?;
            let mut out = Vec::new();
            for c in candidates {
                out.push(parse_candidate(&source_doi, c)?);
            }
            by_doi.insert(source_doi, out);
        }
        Ok(Self { by_doi })
    }

    /// Build the replay extractor from the committed Phase-5 fixture tape.
    pub fn from_fixture() -> Result<Self, String> {
        Self::from_tape(super::FIXTURE_EXTRACTIONS)
    }
}

/// Parse one candidate object off the tape. A confidence outside `[0, 1]`, or a missing
/// required field, is a tape error (fail loud — the tape is committed test data).
fn parse_candidate(source_doi: &str, c: &Value) -> Result<CandidateFinding, String> {
    let verdict = c
        .get("verdict")
        .and_then(Value::as_str)
        .ok_or("candidate: missing verdict")?
        .to_string();
    let confidence = c
        .get("confidence")
        .and_then(Value::as_f64)
        .ok_or("candidate: missing/non-numeric confidence")?;
    if !(0.0..=1.0).contains(&confidence) {
        return Err(format!("candidate: confidence {} out of [0,1]", confidence));
    }
    let assurance = c
        .get("assurance")
        .and_then(Value::as_str)
        .ok_or("candidate: missing assurance")?
        .to_string();
    let justification = c
        .get("justification")
        .and_then(Value::as_str)
        .ok_or("candidate: missing justification")?
        .to_string();
    let cited_dois = c
        .get("cited_dois")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter_map(super::connector::normalise_doi)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(CandidateFinding {
        source_doi: source_doi.to_string(),
        verdict,
        confidence,
        assurance,
        justification,
        cited_dois,
    })
}

impl Extractor for RecordedExtractor {
    fn extract(&self, stubs: &[SourceStub]) -> Result<Vec<CandidateFinding>, String> {
        let mut out = Vec::new();
        for stub in stubs {
            // Replay the recorded candidates for this DOI. A stub with no tape entry yields
            // zero candidates (a DeadEnd source) — that is a valid outcome, not an error.
            if let Some(cands) = self.by_doi.get(&stub.doi) {
                out.extend(cands.iter().cloned());
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::literature::connector::parse_openalex_batch;

    #[test]
    fn recorded_extractor_replays_the_tape_for_each_stub() {
        let (stubs, _) = parse_openalex_batch(crate::literature::FIXTURE_OPENALEX_BATCH).unwrap();
        let extractor = RecordedExtractor::from_fixture().unwrap();
        let cands = extractor.extract(&stubs).unwrap();
        // The fixture has 2 candidates per source × 3 sources = 6 raw candidates.
        assert_eq!(cands.len(), 6);
        // Confidences are all in range (tape validated at parse time).
        assert!(cands.iter().all(|c| (0.0..=1.0).contains(&c.confidence)));
        // One candidate deliberately over-claims Proven (to exercise the SHACL gate later).
        assert!(cands.iter().any(|c| c.assurance == "Proven"));
    }

    #[test]
    fn stub_with_no_tape_entry_yields_no_candidates() {
        let extractor = RecordedExtractor::from_fixture().unwrap();
        let orphan = SourceStub {
            doi: "10.9/not-in-tape".to_string(),
            title: "orphan".to_string(),
            abstract_text: "x".to_string(),
            year: None,
            license: None,
        };
        let cands = extractor.extract(std::slice::from_ref(&orphan)).unwrap();
        assert!(cands.is_empty());
    }

    #[test]
    fn out_of_range_confidence_in_tape_is_an_error() {
        let bad = r#"{ "extractions": [ { "source_doi": "10.1/x", "candidates": [
            { "verdict": "yes", "confidence": 1.5, "assurance": "Conjectured",
              "justification": "twelve chars plus", "cited_dois": [] } ] } ] }"#;
        assert!(RecordedExtractor::from_tape(bad).is_err());
    }
}
