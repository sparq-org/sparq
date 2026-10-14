//! The **live cheap-model batch extractor** behind the `extract::Extractor` trait — the
//! Phase-6 slot the record/replay design left open (§4.3, §4.7). sq-tzars.6
//! (epic `sq-tzars`, design records `research/research-kb-program.md` +
//! `research/provenance-driven-genai-kb.md`). 🤖 SPARQ agent — research-KB live ingestion.
//!
//! ## What this is (and the honesty invariants it holds)
//!
//! `LiveExtractor` turns a batch of `SourceStub`s into candidate `CandidateFinding`s by
//! shelling out to a Claude Code sub-agent (a headless `claude` invocation — no separate
//! Anthropic API key is needed on a box where the CLI is already authenticated, per GH
//! #1139) with a strict JSON-only prompt, then parsing the transcript. It is the ONLY
//! LLM-in-the-loop step, isolated behind the same trait `RecordedExtractor` implements, so:
//!
//! 1. **CI makes ZERO live model calls.** The pure prompt-build / transcript-parse / cap
//!    logic is exercised over COMMITTED recorded transcripts; the subprocess seam
//!    (`CommandRunner`) is behind the default-OFF `literature-live` feature AND is inert
//!    until its command is configured (`SPARQ_KB_EXTRACT_CMD`, default-unset). Record/replay
//!    stays the only test path.
//! 2. **Machine-tier caps are enforced DEFENSIVELY at the boundary** — in ADDITION to the
//!    SHACL gate and the pipeline downgrade, never instead of them. Every parsed candidate
//!    has its confidence CLAMPED to `MACHINE_CONFIDENCE_CEILING` and any `Proven` assurance
//!    MAPPED DOWN to `Claimed` — even if the model output claims otherwise. A candidate
//!    whose justification is not a normalised span of its source abstract is REJECTED at the
//!    boundary as a likely hallucination — and COUNTED (`BoundaryReject`: source DOI +
//!    reason category, no text), never silently dropped, so the pilot's run metrics carry
//!    the fetched→extracted honesty gap (`sq-tx8v9`); the deterministic grounding-resolver
//!    (`ground`) still independently re-checks every survivor — this boundary check never
//!    bypasses it.
//! 3. **Extraction errors surface, never swallowed.** A failed sub-agent invocation, a
//!    transcript with no JSON, or a malformed candidate is an `Err` — never an empty
//!    success. A failed batch quarantines at the pipeline; it never silently zeroes findings.
//!
//! ## The two-layer split (mirrors `connector_core`)
//!
//! - The **pure** layer (`build_batch_prompt`, `parse_transcript`, `extract_json_block`, the
//!   `SubagentRunner` seam, `LiveExtractor`) is behind `literature`. It makes NO subprocess
//!   call — it drives an injected `SubagentRunner`, so CI exercises the whole path over a
//!   fake runner + committed transcripts with ZERO processes spawned.
//! - The **live** layer (`CommandRunner`) is behind `literature-live`. It is the ONLY part
//!   that spawns a process, the command is CONFIGURABLE + default-unset, and it is NEVER
//!   driven in CI.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::Value;

use super::connector::{normalise_doi, SourceStub};
use super::extract::{BoundaryReject, CandidateFinding, Extractor};
use super::ground::normalise_span;
use super::pipeline::MACHINE_CONFIDENCE_CEILING;

/// The **sub-agent invocation seam**. `run` feeds the model the fully-formed JSON-only batch
/// `prompt` and returns its raw transcript (stdout). Abstracted so the live subprocess runner
/// (`CommandRunner`, `literature-live`) and a fixture-replaying fake are interchangeable and
/// CI drives the fake (ZERO processes spawned). An error is surfaced, never swallowed, so a
/// failed batch becomes an `Err` upstream — not an empty success.
pub trait SubagentRunner {
    /// Run one batch and return the raw transcript (the model's stdout). Errors surface.
    fn run(&self, prompt: &str) -> Result<String, String>;
}

/// The **live** extractor: builds a JSON-only batch prompt, runs it through the injected
/// `SubagentRunner`, and parses the transcript into candidate Findings with the defensive
/// machine-tier caps applied at the boundary. Generic over the runner so tests drive it with
/// a fake and production drives it with a `CommandRunner`.
///
/// Every candidate the boundary REJECTS is COUNTED (never silently dropped — `sq-tx8v9`):
/// rejects accumulate across `extract` calls and are reported through
/// [`Extractor::boundary_rejects`], which the pilot threads into the run-metrics sidecar
/// record (source DOI + reason category only — no abstract-derived text).
#[derive(Debug, Clone)]
pub struct LiveExtractor<R: SubagentRunner> {
    runner: R,
    rejects: RefCell<Vec<BoundaryReject>>,
}

impl<R: SubagentRunner> LiveExtractor<R> {
    /// Build a live extractor over an injected sub-agent runner.
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            rejects: RefCell::new(Vec::new()),
        }
    }
}

impl<R: SubagentRunner> Extractor for LiveExtractor<R> {
    /// Extract candidate Findings for the batch. Builds the prompt, runs the sub-agent, and
    /// parses + caps the transcript. A runner failure or an unparseable transcript is an
    /// `Err` (never an empty success); an empty batch is a trivial `Ok(vec![])`.
    fn extract(&self, stubs: &[SourceStub]) -> Result<Vec<CandidateFinding>, String> {
        if stubs.is_empty() {
            return Ok(Vec::new());
        }
        let prompt = build_batch_prompt(stubs);
        let transcript = self.runner.run(&prompt)?;
        let (survivors, rejected) = parse_transcript(stubs, &transcript)?;
        self.rejects.borrow_mut().extend(rejected);
        Ok(survivors)
    }

    /// The boundary rejects accumulated across every `extract` call since construction.
    fn boundary_rejects(&self) -> Vec<BoundaryReject> {
        self.rejects.borrow().clone()
    }
}

/// Build the strict JSON-only batch prompt for a set of source stubs. The prompt pins the
/// exact `{ "extractions": [ … ] }` output shape (the same shape `RecordedExtractor` replays,
/// so a live run's output is a recordable tape), the closed assurance vocabulary (the machine
/// tier may NOT claim `Proven` — the boundary enforces it anyway), the `[0, 1]` confidence
/// range, the load-bearing rule that every justification MUST be a verbatim span of the
/// abstract, and the second-iteration tightening (`sq-tx8v9`, recorded in the first
/// iteration's sidecar verdict): extract ONLY result/conclusion-level claims, never
/// motivation or contribution-framing sentences. Pure + deterministic — no I/O.
pub fn build_batch_prompt(stubs: &[SourceStub]) -> String {
    let mut p = String::with_capacity(2048 + stubs.len() * 512);
    p.push_str(
        "You are a careful research-literature extractor. For EACH paper below, extract the \
         candidate research findings its abstract supports.\n\n\
         Output RULES (a downstream parser reads your reply — obey exactly):\n\
         - Reply with ONE JSON object and NOTHING else. No prose, no explanation.\n\
         - Shape: {\"extractions\":[{\"source_doi\":\"<the paper's source_doi>\",\
         \"candidates\":[{\"verdict\":\"yes|no|partial\",\"confidence\":<number in [0,1]>,\
         \"assurance\":\"Claimed|Conjectured\",\"justification\":\"<a VERBATIM span of THIS \
         paper's abstract>\",\"cited_dois\":[\"<a source_doi present in THIS batch>\"]}]}]}\n\
         - assurance MUST be \"Claimed\" or \"Conjectured\" (a machine extraction may NEVER \
         assert \"Proven\").\n\
         - justification MUST be copied verbatim (character-for-character) from the paper's \
         abstract. Do NOT paraphrase, summarise, or invent. If the abstract does not support \
         a finding, emit an empty candidates list for that paper.\n\
         - Extract ONLY result/conclusion-level claims (what the paper found, showed, \
         demonstrated, or concluded). Do NOT extract motivation, background, or \
         contribution-framing sentences (\"we propose\", \"this paper presents\", \
         \"X is important\").\n\
         - cited_dois must each be a source_doi that appears in this batch.\n\n\
         PAPERS:\n",
    );
    for (i, s) in stubs.iter().enumerate() {
        // POSITIONAL format args (CodeQL rust/unused-variable false-positive guard).
        p.push_str(&format!(
            "[{}] source_doi: {}\n    title: {}\n    abstract: {}\n\n",
            i + 1,
            s.source_iri(),
            s.title,
            s.abstract_text,
        ));
    }
    p
}

/// Parse a raw sub-agent transcript into `(survivors, boundary rejects)` for the batch,
/// applying the DEFENSIVE machine-tier caps at the boundary. The transcript may wrap its
/// JSON in prose or a fenced block; `extract_json_block` recovers the JSON object. A
/// transcript with no JSON object, or a candidate missing a required field, is an `Err`
/// (a failed batch is never a silent empty success).
///
/// Caps applied to every parsed candidate (defence-in-depth — the pipeline's grounding stage
/// + SHACL gate + downgrade still run):
/// - confidence is clamped to `[0, MACHINE_CONFIDENCE_CEILING]` (a NaN becomes `0.0`);
/// - any `Proven` assurance is mapped down to `Claimed`;
/// - a candidate whose justification is not a normalised span of its source abstract (or
///   whose source_doi is not in the batch) is REJECTED as a likely hallucination — and
///   COUNTED in the returned [`BoundaryReject`] list (source DOI + reason category, no
///   text), never silently dropped (`sq-tx8v9`).
pub fn parse_transcript(
    stubs: &[SourceStub],
    transcript: &str,
) -> Result<(Vec<CandidateFinding>, Vec<BoundaryReject>), String> {
    let json = extract_json_block(transcript)?;
    let root: Value =
        serde_json::from_str(json).map_err(|e| format!("live extraction JSON: {}", e))?;
    let entries = root
        .get("extractions")
        .and_then(Value::as_array)
        .ok_or_else(|| "live extraction: missing `extractions` array".to_string())?;

    // Abstract-by-DOI index for the boundary verbatim-span check.
    let mut abstract_by_doi: HashMap<&str, &str> = HashMap::with_capacity(stubs.len());
    for s in stubs {
        abstract_by_doi.insert(s.doi.as_str(), s.abstract_text.as_str());
    }

    let mut out = Vec::new();
    let mut rejected = Vec::new();
    for entry in entries {
        let source_doi = entry
            .get("source_doi")
            .and_then(Value::as_str)
            .and_then(normalise_doi)
            .ok_or_else(|| "live extraction: an entry has no source_doi".to_string())?;
        let candidates = entry
            .get("candidates")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!(
                    "live extraction: entry {} has no candidates array",
                    source_doi
                )
            })?;
        for c in candidates {
            let raw = parse_live_candidate(&source_doi, c)?;
            match apply_defensive_caps(raw, &abstract_by_doi) {
                Ok(capped) => out.push(capped),
                Err(reject) => rejected.push(reject),
            }
        }
    }
    Ok((out, rejected))
}

/// Parse ONE candidate object off the live transcript. A missing/mistyped required field is
/// an `Err` (malformed-transcript rejection). Unlike the recorded-tape parser this is LENIENT
/// on the confidence RANGE — an out-of-range value from an untrusted model is clamped by
/// `apply_defensive_caps`, not rejected — but STRICT on presence + type.
fn parse_live_candidate(source_doi: &str, c: &Value) -> Result<CandidateFinding, String> {
    let verdict = c
        .get("verdict")
        .and_then(Value::as_str)
        .ok_or("live candidate: missing verdict")?
        .to_string();
    let confidence = c
        .get("confidence")
        .and_then(Value::as_f64)
        .ok_or("live candidate: missing/non-numeric confidence")?;
    let assurance = c
        .get("assurance")
        .and_then(Value::as_str)
        .ok_or("live candidate: missing assurance")?
        .to_string();
    let justification = c
        .get("justification")
        .and_then(Value::as_str)
        .ok_or("live candidate: missing justification")?
        .to_string();
    let cited_dois = c
        .get("cited_dois")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter_map(normalise_doi)
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

/// Apply the DEFENSIVE machine-tier caps to a raw candidate at the extraction boundary.
/// Returns `Err(BoundaryReject)` — the claimed source DOI plus a stable reason category,
/// NEVER the justification text — if the candidate must be REJECTED (its source is not in
/// the batch, or its justification is not a normalised span of that source's abstract — a
/// likely hallucination), so the caller can COUNT the drop (`sq-tx8v9`). Otherwise returns
/// the candidate with confidence clamped to the machine ceiling and any `Proven` assurance
/// mapped down to `Claimed`. Defence-in-depth: the pipeline's grounding stage independently
/// re-checks every survivor (never bypassed) and the SHACL gate + emit downgrade enforce
/// `secx:Conjectured` + the ceiling again.
fn apply_defensive_caps(
    mut c: CandidateFinding,
    abstract_by_doi: &HashMap<&str, &str>,
) -> Result<CandidateFinding, BoundaryReject> {
    // 1. REJECT a candidate whose justification is not a verbatim (normalised) span of its
    //    source abstract, or whose source is not in the batch (cannot be anchored). The
    //    reason categories are the same stable keys the pilot's quarantine metrics use.
    let Some(abstract_text) = abstract_by_doi.get(c.source_doi.as_str()) else {
        return Err(BoundaryReject {
            source_doi: c.source_doi,
            reason: "unknown-source".to_string(),
        });
    };
    let hay = normalise_span(abstract_text);
    let needle = normalise_span(&c.justification);
    if needle.is_empty() || !hay.contains(&needle) {
        return Err(BoundaryReject {
            source_doi: c.source_doi,
            reason: "justification-not-anchored".to_string(),
        });
    }
    // 2. CLAMP confidence to the machine ceiling (also normalises an out-of-range / NaN value
    //    from the untrusted model).
    c.confidence = if c.confidence.is_nan() {
        0.0
    } else {
        c.confidence.clamp(0.0, MACHINE_CONFIDENCE_CEILING)
    };
    // 3. Never let `Proven` through — a machine extraction may not outrank a human.
    if c.assurance.eq_ignore_ascii_case("Proven") {
        c.assurance = "Claimed".to_string();
    }
    Ok(c)
}

/// Recover the JSON object from a raw model transcript. The sub-agent is instructed to emit
/// ONLY a JSON object, but a real model may wrap it in a fenced ```` ```json ```` block or
/// surrounding prose; this prefers a fenced block, else takes the span from the first `{` to
/// the last `}`. Errors if no `{ … }` is present (a failed batch surfaces, never a silent
/// empty success).
pub fn extract_json_block(transcript: &str) -> Result<&str, String> {
    if let Some(block) = fenced_json_block(transcript) {
        return Ok(block);
    }
    let start = transcript
        .find('{')
        .ok_or_else(|| "live extraction: no JSON object in transcript".to_string())?;
    let end = transcript
        .rfind('}')
        .ok_or_else(|| "live extraction: no closing brace in transcript".to_string())?;
    if end < start {
        return Err("live extraction: malformed JSON braces in transcript".to_string());
    }
    Ok(&transcript[start..=end])
}

/// Extract the body of the first fenced code block (```` ``` ```` … ```` ``` ````) when it
/// contains a JSON object. Returns `None` (fall back to a brace scan) when there is no fence,
/// no closing fence, or the fenced body is not a JSON object.
fn fenced_json_block(transcript: &str) -> Option<&str> {
    let open = transcript.find("```")?;
    let after = &transcript[open + 3..];
    // Skip an optional language tag on the fence line (e.g. `json`).
    let body_start = after.find('\n')? + 1;
    let body = &after[body_start..];
    let close = body.find("```")?;
    let inner = body[..close].trim();
    if inner.starts_with('{') {
        Some(inner)
    } else {
        None
    }
}

// --------------------------------------------------------------------------------------
// Live subprocess seam (feature = "literature-live") — the ONLY part that spawns a process,
// CONFIGURABLE + default-unset, NEVER driven in CI.
// --------------------------------------------------------------------------------------

/// The environment variable that configures the live sub-agent command (default-UNSET, so the
/// wiring is real but INERT without explicit opt-in). Its value is a whitespace-separated
/// command line, e.g. `claude -p --model claude-haiku-4-5 --output-format text`; the batch
/// prompt is written to the child's stdin and its stdout is the transcript.
#[cfg(feature = "literature-live")]
pub const EXTRACT_CMD_ENV: &str = "SPARQ_KB_EXTRACT_CMD";

/// The live `SubagentRunner`: spawns a configured command, writes the prompt to its stdin,
/// and returns its stdout. NETWORK/PROCESS — never driven in CI (the pure path is tested over
/// committed transcripts + a fake runner).
///
/// The command is read from `EXTRACT_CMD_ENV` (`CommandRunner::from_env`) or supplied directly
/// (`CommandRunner::from_command`). Whitespace-separated argv only (no shell, no quoting) — the
/// command is executed directly, so there is no shell-injection surface; a command needing
/// spaces inside a single argument is out of scope for this v1 seam.
#[cfg(feature = "literature-live")]
#[derive(Debug, Clone)]
pub struct CommandRunner {
    program: String,
    args: Vec<String>,
}

#[cfg(feature = "literature-live")]
impl CommandRunner {
    /// Build the runner from the `EXTRACT_CMD_ENV` (`SPARQ_KB_EXTRACT_CMD`) environment
    /// variable. Errors if it is unset or blank — the seam is INERT until explicitly
    /// configured, so a live extraction never runs by accident.
    pub fn from_env() -> Result<Self, String> {
        let cmd = std::env::var(EXTRACT_CMD_ENV).map_err(|_| {
            // POSITIONAL format args (CodeQL rust/unused-variable false-positive guard).
            format!(
                "live extractor: {} is not set (the live sub-agent command is default-unset; \
                 set it to opt in, e.g. `claude -p --model claude-haiku-4-5`)",
                EXTRACT_CMD_ENV
            )
        })?;
        Self::from_command(&cmd)
    }

    /// Build the runner from a whitespace-separated command line. Errors if it is blank.
    pub fn from_command(cmd: &str) -> Result<Self, String> {
        let mut parts = cmd.split_whitespace().map(str::to_string);
        let program = parts
            .next()
            .ok_or_else(|| "live extractor: empty command".to_string())?;
        let args: Vec<String> = parts.collect();
        Ok(Self { program, args })
    }

    /// The resolved program (`argv[0]`).
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The resolved arguments (`argv[1..]`).
    pub fn args(&self) -> &[String] {
        &self.args
    }
}

#[cfg(feature = "literature-live")]
impl SubagentRunner for CommandRunner {
    fn run(&self, prompt: &str) -> Result<String, String> {
        use std::io::Write as _;
        use std::process::{Command, Stdio};

        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("live extractor: failed to spawn `{}`: {}", self.program, e))?;

        // Write the prompt on a separate thread so a child that streams a large stdout while
        // still reading a large stdin cannot deadlock on the OS pipe buffer. A broken-pipe
        // write error (a child that exits early) is intentionally ignored — the exit-status
        // check below is the authoritative failure signal.
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "live extractor: child stdin unavailable".to_string())?;
        let prompt_owned = prompt.to_string();
        let writer = std::thread::spawn(move || {
            let mut stdin = stdin;
            let _ = stdin.write_all(prompt_owned.as_bytes());
        });

        let output = child
            .wait_with_output()
            .map_err(|e| format!("live extractor: waiting on child failed: {}", e))?;
        let _ = writer.join();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let truncated: String = stderr.chars().take(500).collect();
            return Err(format!(
                "live extractor: `{}` exited unsuccessfully ({}): {}",
                self.program, output.status, truncated
            ));
        }
        String::from_utf8(output.stdout)
            .map_err(|e| format!("live extractor: child stdout was not UTF-8: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::literature::connector::parse_openalex_batch;
    use crate::literature::extract::RecordedExtractor;

    /// The committed recorded sub-agent transcript (a FABRICATED example of a cheap model's
    /// raw stdout for the `openalex-batch.json` papers). Replayed so tests make ZERO model
    /// calls; deliberately adversarial to exercise the defensive caps.
    const TRANSCRIPT: &str =
        include_str!("../../fixtures/literature/extract-live-transcript.txt");

    fn stubs() -> Vec<SourceStub> {
        let (stubs, _) = parse_openalex_batch(crate::literature::FIXTURE_OPENALEX_BATCH).unwrap();
        stubs
    }

    /// A fake runner that replays a scripted result — no subprocess, ZERO model calls.
    struct FakeRunner(Result<String, String>);
    impl SubagentRunner for FakeRunner {
        fn run(&self, _prompt: &str) -> Result<String, String> {
            self.0.clone()
        }
    }

    // ---- build_batch_prompt -----------------------------------------------------------

    #[test]
    fn build_batch_prompt_is_json_only_and_lists_every_source() {
        let stubs = stubs();
        let p = build_batch_prompt(&stubs);
        assert!(p.contains("ONE JSON object"), "pins JSON-only output");
        assert!(p.contains("\"extractions\""), "pins the tape shape");
        assert!(
            p.contains("may NEVER assert \"Proven\""),
            "instructs the closed no-Proven vocabulary"
        );
        assert!(p.contains("VERBATIM span"), "instructs verbatim justification");
        assert!(
            p.contains("result/conclusion-level claims"),
            "instructs the second-iteration tightening: results/conclusions only"
        );
        assert!(
            p.contains("Do NOT extract motivation"),
            "excludes motivation/contribution-framing sentences"
        );
        // Every source stub appears (its content-addressed IRI + abstract).
        for s in &stubs {
            assert!(p.contains(&s.source_iri()), "prompt lists {}", s.source_iri());
            assert!(p.contains(&s.abstract_text), "prompt carries the abstract");
        }
    }

    // ---- extract_json_block -----------------------------------------------------------

    #[test]
    fn extract_json_block_handles_clean_fenced_prose_and_missing() {
        // Clean JSON.
        assert_eq!(
            extract_json_block(r#"{"extractions":[]}"#).unwrap(),
            r#"{"extractions":[]}"#
        );
        // Fenced ```json block wrapped in prose.
        let fenced = "here you go:\n```json\n{\"extractions\":[]}\n```\nthanks!";
        assert_eq!(extract_json_block(fenced).unwrap(), r#"{"extractions":[]}"#);
        // Prose-wrapped, no fence (first `{` to last `}`).
        let prose = "Sure: {\"extractions\":[]} done";
        assert_eq!(extract_json_block(prose).unwrap(), r#"{"extractions":[]}"#);
        // No JSON at all is an error (a failed batch surfaces).
        assert!(extract_json_block("no json here").is_err());
    }

    // ---- parse_transcript: the defensive caps -----------------------------------------

    #[test]
    fn parse_transcript_clamps_confidence_and_downgrades_proven() {
        let stubs = stubs();
        let (cands, rejects) = parse_transcript(&stubs, TRANSCRIPT).unwrap();
        // 4 raw candidates; the fabricated "machine-checked soundness theorem" (paper 2) is
        // REJECTED at the boundary (not a span) -> 3 survivors.
        assert_eq!(cands.len(), 3, "the fabricated over-claim is rejected");
        // INVARIANT (sq-tx8v9): the boundary drop is COUNTED, reason-tagged, and carries
        // NO justification text.
        assert_eq!(rejects.len(), 1, "the drop is counted, never silent");
        assert_eq!(rejects[0].reason, "justification-not-anchored");
        assert!(!rejects[0].source_doi.is_empty());
        // INVARIANT: no survivor asserts Proven, whatever the model claimed.
        assert!(
            cands.iter().all(|c| !c.assurance.eq_ignore_ascii_case("Proven")),
            "no survivor is Proven"
        );
        // INVARIANT: every survivor's confidence is clamped to the machine ceiling.
        assert!(
            cands
                .iter()
                .all(|c| c.confidence <= MACHINE_CONFIDENCE_CEILING),
            "all clamped to <= ceiling"
        );
        // The over-claiming candidate (Proven, 0.95) survived (its justification IS a span),
        // downgraded to Claimed with confidence clamped to exactly the ceiling.
        let capped = cands
            .iter()
            .find(|c| c.justification.starts_with("Blank-node and prefix scope"))
            .expect("the over-claim survives (its justification is a span)");
        assert_eq!(capped.assurance, "Claimed", "Proven mapped down to Claimed");
        assert_eq!(
            capped.confidence, MACHINE_CONFIDENCE_CEILING,
            "0.95 clamped to the ceiling"
        );
        // The fabricated over-claim is gone.
        assert!(
            !cands
                .iter()
                .any(|c| c.justification.contains("machine-checked soundness theorem")),
            "the fabricated justification was rejected"
        );
    }

    #[test]
    fn parse_transcript_rejects_non_span_justification() {
        // A justification that is NOT a span of the (single) source abstract is dropped.
        let stub = SourceStub {
            doi: "10.1/x".to_string(),
            title: "T".to_string(),
            abstract_text: "the abstract says exactly this and nothing else".to_string(),
            year: None,
            license: None,
        };
        let json = r#"{"extractions":[{"source_doi":"10.1/x","candidates":[
            {"verdict":"yes","confidence":0.5,"assurance":"Conjectured",
             "justification":"a fabricated claim absent from the abstract","cited_dois":[]},
            {"verdict":"yes","confidence":0.5,"assurance":"Conjectured",
             "justification":"exactly this","cited_dois":[]}
        ]}]}"#;
        let (cands, rejects) = parse_transcript(std::slice::from_ref(&stub), json).unwrap();
        assert_eq!(cands.len(), 1, "only the in-abstract span survives");
        assert_eq!(cands[0].justification, "exactly this");
        // The fabricated candidate is counted with its reason (sq-tx8v9).
        assert_eq!(rejects.len(), 1);
        assert_eq!(rejects[0].source_doi, "10.1/x");
        assert_eq!(rejects[0].reason, "justification-not-anchored");
    }

    #[test]
    fn parse_transcript_counts_an_unknown_source_reject() {
        // A candidate whose source_doi is not in the batch cannot be anchored: rejected
        // AND counted under the `unknown-source` category.
        let stub = SourceStub {
            doi: "10.1/x".to_string(),
            title: "T".to_string(),
            abstract_text: "the abstract".to_string(),
            year: None,
            license: None,
        };
        let json = r#"{"extractions":[{"source_doi":"10.9/not-in-batch","candidates":[
            {"verdict":"yes","confidence":0.5,"assurance":"Conjectured",
             "justification":"the abstract","cited_dois":[]}
        ]}]}"#;
        let (cands, rejects) = parse_transcript(std::slice::from_ref(&stub), json).unwrap();
        assert!(cands.is_empty());
        assert_eq!(rejects.len(), 1);
        assert_eq!(rejects[0].source_doi, "10.9/not-in-batch");
        assert_eq!(rejects[0].reason, "unknown-source");
    }

    #[test]
    fn parse_transcript_clamps_out_of_range_confidence() {
        let stub = SourceStub {
            doi: "10.1/x".to_string(),
            title: "T".to_string(),
            abstract_text: "an anchored span here".to_string(),
            year: None,
            license: None,
        };
        // Confidence 4.2 is out of [0,1] — LIVE parse clamps it (untrusted model), not errors.
        let json = r#"{"extractions":[{"source_doi":"10.1/x","candidates":[
            {"verdict":"yes","confidence":4.2,"assurance":"Conjectured",
             "justification":"an anchored span here","cited_dois":[]}
        ]}]}"#;
        let (cands, rejects) = parse_transcript(std::slice::from_ref(&stub), json).unwrap();
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].confidence, MACHINE_CONFIDENCE_CEILING);
        assert!(rejects.is_empty(), "a clamped survivor is not a reject");
    }

    #[test]
    fn parse_transcript_rejects_malformed_json_and_missing_fields() {
        let stubs = stubs();
        // No JSON object at all.
        assert!(parse_transcript(&stubs, "the model refused").is_err());
        // Valid JSON but a candidate missing `confidence` -> Err (never a silent success).
        let missing = r#"{"extractions":[{"source_doi":"10.1/x","candidates":[
            {"verdict":"yes","assurance":"Conjectured","justification":"x","cited_dois":[]}
        ]}]}"#;
        assert!(parse_transcript(&stubs, missing).is_err());
    }

    // ---- LiveExtractor over a fake runner ---------------------------------------------

    #[test]
    fn live_extractor_extract_runs_the_runner_and_caps() {
        let stubs = stubs();
        let ex = LiveExtractor::new(FakeRunner(Ok(TRANSCRIPT.to_string())));
        let cands = ex.extract(&stubs).unwrap();
        assert_eq!(cands.len(), 3, "same 3 capped survivors as parse_transcript");
        assert!(cands.iter().all(|c| c.confidence <= MACHINE_CONFIDENCE_CEILING));
        // The boundary reject is reported through the trait (sq-tx8v9)…
        let rejects = ex.boundary_rejects();
        assert_eq!(rejects.len(), 1, "the boundary drop is counted");
        assert_eq!(rejects[0].reason, "justification-not-anchored");
        // …and ACCUMULATES across extract calls (the pilot extracts in batches).
        ex.extract(&stubs).unwrap();
        assert_eq!(ex.boundary_rejects().len(), 2, "rejects accumulate per call");
    }

    #[test]
    fn live_extractor_surfaces_a_runner_error() {
        let stubs = stubs();
        let ex = LiveExtractor::new(FakeRunner(Err("sub-agent process died".to_string())));
        // A failed batch is an Err, NEVER an empty success.
        let err = ex.extract(&stubs).unwrap_err();
        assert!(err.contains("process died"));
    }

    #[test]
    fn live_extractor_failed_batch_is_err_not_empty() {
        let stubs = stubs();
        // The runner "succeeds" but returns junk with no JSON -> Err, not Ok(vec![]).
        let ex = LiveExtractor::new(FakeRunner(Ok("I could not comply".to_string())));
        assert!(ex.extract(&stubs).is_err(), "no-JSON transcript is an error");
    }

    #[test]
    fn live_extractor_empty_batch_is_trivially_ok() {
        let ex = LiveExtractor::new(FakeRunner(Err("must not be called".to_string())));
        assert_eq!(ex.extract(&[]).unwrap(), Vec::new());
    }

    // ---- record/replay honesty: the transcript's JSON IS a replayable tape ------------

    #[test]
    fn transcript_json_replays_as_a_recorded_tape() {
        // The RAW (pre-cap) JSON block extracted from a live transcript is a valid tape:
        // RecordedExtractor::from_tape parses it unchanged, so a live run regenerates
        // fixtures honestly (all 4 raw candidates, including the ones the boundary caps/drops).
        let json = extract_json_block(TRANSCRIPT).unwrap();
        let replay = RecordedExtractor::from_tape(json).expect("transcript JSON is a valid tape");
        let stubs = stubs();
        let raw = replay.extract(&stubs).unwrap();
        assert_eq!(raw.len(), 4, "the raw tape keeps every candidate (no caps applied)");
        assert!(
            raw.iter().any(|c| c.assurance == "Proven"),
            "the raw tape preserves the model's Proven over-claim (caps live in LiveExtractor)"
        );
    }

    // ---- CommandRunner (feature = "literature-live") — no model calls ------------------

    #[cfg(feature = "literature-live")]
    mod live_runner {
        use super::*;

        #[test]
        fn from_command_splits_argv() {
            let r = CommandRunner::from_command("claude -p --model claude-haiku-4-5").unwrap();
            assert_eq!(r.program(), "claude");
            assert_eq!(r.args(), &["-p", "--model", "claude-haiku-4-5"]);
        }

        #[test]
        fn from_command_blank_is_err() {
            assert!(CommandRunner::from_command("   ").is_err());
        }

        #[test]
        fn command_runner_pipes_stdin_to_stdout() {
            // `cat` echoes the prompt on stdin back to stdout — a REAL subprocess, ZERO model.
            let r = CommandRunner::from_command("cat").unwrap();
            assert_eq!(r.run("hello prompt").unwrap(), "hello prompt");
        }

        #[test]
        fn command_runner_surfaces_a_nonzero_exit() {
            // `false` exits 1 with no output -> the runner surfaces an error, never a silent
            // empty success.
            let r = CommandRunner::from_command("false").unwrap();
            assert!(r.run("x").is_err());
        }

        #[test]
        fn from_env_reads_the_configurable_default_unset_command() {
            // Confined env mutation — only this test touches EXTRACT_CMD_ENV. Save + restore.
            let saved = std::env::var(EXTRACT_CMD_ENV).ok();

            std::env::remove_var(EXTRACT_CMD_ENV);
            assert!(
                CommandRunner::from_env().is_err(),
                "default-unset => the seam is inert => Err"
            );

            std::env::set_var(EXTRACT_CMD_ENV, "cat");
            let r = CommandRunner::from_env().expect("a set command constructs the runner");
            assert_eq!(r.run("ping").unwrap(), "ping", "the configured command runs");

            match saved {
                Some(v) => std::env::set_var(EXTRACT_CMD_ENV, v),
                None => std::env::remove_var(EXTRACT_CMD_ENV),
            }
        }
    }
}
