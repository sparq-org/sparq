//! The **pipeline** — wires `[connector] -> [normalise] -> [extract (replay)] ->
//! [ground] -> [emit TTL] -> [sidecar]` over a committed fixture batch, with ZERO network
//! and ZERO live-model calls. (§4.7, the projector that extends `ingest_pkg.py` — realised
//! in Rust behind the `literature` feature.)
//!
//! [OPUS-4.8] sq-2489d.5. The pipeline is the Phase-5 deliverable: it produces (a) a
//! Turtle document of the *grounded, machine-tier* `pkg:Source` + `pkg:Finding` triples
//! with full PROV-O lineage, and (b) a [`Sidecar`] reporting the citation-grounding rate +
//! the quarantine list. The caller runs the SHACL gate (`pkg.shapes.ttl` +
//! `shapes/literature.shapes.ttl`) over the emitted TTL to obtain the SHACL-conformance
//! rate — see `tests/literature_pipeline.rs`.
//!
//! The committed Findings are deliberately NOT appended to `ingest/pkg-instances.ttl`:
//! Phase 5 ships the *scaffolding* and proves it on fixtures; bulk ingestion of real
//! literature is gated behind the Phase-6 live pilot's per-topic recommend-adopt verdict.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::connector::{parse_openalex_batch, SourceStub};
use super::extract::{CandidateFinding, Extractor};
use super::ground::{self, GroundingFailure};
use crate::vocab::{TIER_LICENSE_RESTRICTED_GRAPH, TIER_MACHINE_GRAPH};

/// The machine-extraction agent IRI every emitted Finding is `prov:wasAttributedTo`. Typed
/// `pkg:MachineAgent`, so the literature-tier SHACL shapes bind to it (and a hand-authored
/// Finding, which is NOT attributed to it, is unconstrained by those shapes).
pub const MACHINE_AGENT_IRI: &str = "https://sparq.dev/ns/pkg/agent#literature-extractor";

/// The confidence ceiling the machine tier is capped at (mirrors the
/// `shapes/literature.shapes.ttl` RULE 2). A candidate proposing more is clamped to this
/// at emit time so the emitted TTL is conformant by construction; the SHACL shape is the
/// durable backstop. NOT a calibrated number (calibration is Phase 6) — a declarative
/// "a cheap extractor is not high-confidence" guard.
pub const MACHINE_CONFIDENCE_CEILING: f64 = 0.7;

/// The named tier a KB statement is emitted into (`sq-tzars.7`). Tier separation v1 is
/// realised as per-tier Turtle ARTIFACTS (not in-store named graphs); each variant NAMES
/// a `pkg/graph#` IRI ([`Tier::graph_iri`]). Every emitted machine-pipeline statement
/// belongs to EXACTLY ONE tier — the load-bearing invariant of `sq-tzars.7`.
///
/// The pipeline itself only ever emits [`Tier::Machine`] and [`Tier::LicenseRestricted`]
/// (the machine-extraction tiers); [`Tier::HandAuthored`] names the `ingest_pkg.py`
/// projector output, which is stamped with its graph IRI by that script, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    /// The hand-authored, human-curated PKG (`ingest_pkg.py` output).
    HandAuthored,
    /// Machine findings from a KNOWN, redistributable-licensed source.
    Machine,
    /// Machine findings from an unknown/absent/non-redistributable-licensed source
    /// (fail-closed). The full findings stay private; only a metadata-only projection is
    /// published.
    LicenseRestricted,
}

impl Tier {
    /// The `pkg/graph#` named-graph IRI for this tier (a byte-pinned `crate::vocab`
    /// constant). Downstream artifacts + the export script (`sq-tzars.8`) key off it.
    pub fn graph_iri(&self) -> &'static str {
        match self {
            Tier::HandAuthored => crate::vocab::TIER_HAND_AUTHORED_GRAPH,
            Tier::Machine => TIER_MACHINE_GRAPH,
            Tier::LicenseRestricted => TIER_LICENSE_RESTRICTED_GRAPH,
        }
    }
}

/// Route a machine-pipeline source to its emission tier from its captured licence,
/// **fail-closed**: an UNKNOWN/absent licence (`None`), a blank licence, or any licence
/// NOT on the conservative redistributable allowlist routes to [`Tier::LicenseRestricted`];
/// only a licence on the allowlist routes to [`Tier::Machine`]. This is the load-bearing
/// invariant of `sq-tzars.7`: unknown licence ⇒ restricted.
///
/// The allowlist is the clearly-redistributable open set (`cc-by`, `cc-by-sa`, `cc0`,
/// public-domain); `-nc`/`-nd` variants and anything unrecognised are treated as
/// non-redistributable (a public dump is a redistribution, so the safe default is to
/// restrict). It is a deliberately conservative, maintainer-tunable policy — see the crate
/// README + the `sq-tzars.7` PR body.
pub fn source_tier(license: Option<&str>) -> Tier {
    match license.map(str::trim).filter(|s| !s.is_empty()) {
        Some(l) if is_redistributable_license(l) => Tier::Machine,
        _ => Tier::LicenseRestricted,
    }
}

/// The conservative redistributable-licence allowlist (case-insensitive exact match).
/// Anything not listed — including an unknown/absent licence — is treated as
/// non-redistributable by [`source_tier`] (fail-closed).
fn is_redistributable_license(license: &str) -> bool {
    const ALLOW: &[&str] = &[
        "cc-by",
        "cc-by-sa",
        "cc0",
        "cc-0",
        "public-domain",
        "publicdomain",
        "pd",
    ];
    let l = license.to_ascii_lowercase();
    ALLOW.contains(&l.as_str())
}

/// The metadata-only public licence-status literal for a restricted source: the raw
/// licence string when one is present (e.g. a known-but-non-redistributable `cc-by-nc`),
/// else `"unknown"` for an absent/blank licence. Carries NO abstract-derived text.
fn license_status(license: Option<&str>) -> String {
    match license.map(str::trim).filter(|s| !s.is_empty()) {
        Some(l) => l.to_string(),
        None => "unknown".to_string(),
    }
}

/// Whether the connector batch feeding the pipeline was COMPLETE or was cut short by a
/// pagination hard-cap. A cap-truncated batch (a paged connector run whose
/// `pages_fetched` reached the policy `max_pages`) may be missing results, so an artifact
/// emitted from it MUST NOT be presented as the whole result set (`sq-tzars.7` truncation
/// invariant; the consumer note from the #1527 review). Fail-closed: "hit the cap" is
/// treated as potentially-incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchCompleteness {
    /// The connector exhausted the result set (an empty/under-full page arrived, or the
    /// reported `totalHits` was reached before the cap).
    Complete,
    /// The connector stopped at a hard page cap — there may be MORE unfetched results.
    CapTruncated {
        /// Pages actually fetched (`== max_pages` when the cap was the stop reason).
        pages_fetched: usize,
        /// The policy page cap that was hit.
        max_pages: usize,
    },
}

impl BatchCompleteness {
    /// Derive completeness from a paginated connector run's `pages_fetched` vs the policy
    /// page cap (e.g. a `connector_core::CoreSearchResult.pages_fetched` and
    /// `RetryPolicy.max_pages`). `pages_fetched >= max_pages` (for a positive cap) ⇒
    /// [`BatchCompleteness::CapTruncated`] — the run may have stopped short of the full
    /// result set. This decouples the pipeline from the connector: the caller maps its
    /// run into this without the pipeline importing the connector's paging types.
    pub fn from_pagination(pages_fetched: usize, max_pages: usize) -> Self {
        if max_pages > 0 && pages_fetched >= max_pages {
            BatchCompleteness::CapTruncated {
                pages_fetched,
                max_pages,
            }
        } else {
            BatchCompleteness::Complete
        }
    }

    /// Whether the batch is known-complete.
    pub fn is_complete(&self) -> bool {
        matches!(self, BatchCompleteness::Complete)
    }
}

/// One quarantined candidate (recorded, never silently dropped — the sidecar-honesty
/// pattern §4.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quarantined {
    /// The source DOI the quarantined candidate came from.
    pub source_doi: String,
    /// The (truncated) justification, for human triage.
    pub justification: String,
    /// Why it was quarantined.
    pub reason: String,
}

/// The pipeline sidecar — the Phase-5 metric carrier. Reports the citation-grounding rate
/// and the quarantine list for one fixture batch. No hard-coded number lives here: the
/// rate is computed from the batch at run time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sidecar {
    /// Total candidate Findings the extractor proposed across all sources.
    pub candidates_total: usize,
    /// Candidates that PASSED the deterministic grounding-resolver (committed to the TTL).
    pub grounded: usize,
    /// Candidates QUARANTINED by grounding (recorded here, not dropped).
    pub quarantined: Vec<Quarantined>,
    /// Sources the connector skipped (no DOI / no title) — surfaced, not lost.
    pub sources_skipped: usize,
    /// Distinct sources that produced ≥1 grounded Finding (flipped to `pkg:Explored`).
    pub sources_explored: usize,
    /// Distinct sources that produced 0 grounded Findings (`pkg:DeadEnd`).
    pub sources_dead_end: usize,
}

impl Sidecar {
    /// The **citation-grounding rate**: grounded candidates / total candidates, in `[0,1]`.
    /// `1.0` when there were no candidates (vacuously grounded — nothing ungrounded slipped
    /// through). This is one of the two Phase-5 acceptance metrics.
    pub fn grounding_rate(&self) -> f64 {
        if self.candidates_total == 0 {
            return 1.0;
        }
        self.grounded as f64 / self.candidates_total as f64
    }

    /// The **source-yield rate**: sources that produced ≥1 grounded Finding
    /// (`pkg:Explored`) / sources the connector parsed, in `[0,1]`. `1.0` when the batch
    /// parsed no sources (vacuous — nothing was wasted), mirroring
    /// [`grounding_rate`](Self::grounding_rate)'s empty-batch convention.
    ///
    /// This is a **targeting**-quality signal (how well the connector query selected
    /// usable sources), NOT a quality signal for any individual Finding, and it is
    /// deliberately computed over parsed sources only — `sources_skipped` (no DOI/title)
    /// never reached extraction, so folding it in here would conflate connector-input
    /// hygiene with extraction yield.
    pub fn source_yield_rate(&self) -> f64 {
        let sources = self.sources_explored + self.sources_dead_end;
        if sources == 0 {
            return 1.0;
        }
        self.sources_explored as f64 / sources as f64
    }

    /// Emit this run's batch-level quality as **DQV `dqv:QualityMeasurement` triples** —
    /// the §4.5 "how good is this batch?" surface, so the run verdict is a *query* (the
    /// `batch-quality` canned query) rather than a log line only.
    ///
    /// The document is standalone (its own `@prefix` block) and is emitted SEPARATELY
    /// from the finding artifacts: the batch metrics describe the *run*, not the graph's
    /// content, so a consumer can load the findings without the run telemetry, or the
    /// telemetry alone to compare batches over time.
    ///
    /// `batch_iri` names the run (see [`batch_iri`]); each measurement hangs off it as
    /// `{batch_iri}/quality/{metric}`. Only metrics this pipeline actually computes are
    /// emitted — see [`crate::vocab::BATCH_QUALITY_DIMENSION`] for why the design's
    /// audit-, dedup- and topic-coverage metrics are absent rather than emitted as zero.
    ///
    /// **Honest reading of these numbers:** both are *structural* rates. A grounding rate
    /// of 1.0 means every committed Finding is anchored in its source text and cites only
    /// in-batch sources; it does **not** mean the Findings are true. Extraction accuracy
    /// is unmeasured until the Phase-6 human-audited sample.
    pub fn quality_measurements_turtle(
        &self,
        batch_iri: &str,
        generated_at_time: &str,
    ) -> Result<String, String> {
        let mut t = String::from(QUALITY_TTL_PREFIXES);
        writeln!(
            t,
            "<{batch}> a prov:Activity ;\n  \
             rdfs:label \"literature ingestion batch\"@en ;\n  \
             prov:wasAssociatedWith <{agent}> ;\n  \
             prov:generatedAtTime \"{ts}\"^^xsd:dateTime ;\n  \
             dqv:hasQualityMeasurement <{batch}/quality/grounding-rate> , \
             <{batch}/quality/source-yield-rate> .",
            batch = batch_iri,
            agent = MACHINE_AGENT_IRI,
            ts = generated_at_time,
        )
        .map_err(|e| e.to_string())?;
        for (slug, metric, value) in [
            (
                "grounding-rate",
                crate::vocab::GROUNDING_RATE_METRIC,
                self.grounding_rate(),
            ),
            (
                "source-yield-rate",
                crate::vocab::SOURCE_YIELD_RATE_METRIC,
                self.source_yield_rate(),
            ),
        ] {
            writeln!(
                t,
                "\n<{batch}/quality/{slug}> a dqv:QualityMeasurement ;\n  \
                 dqv:isMeasurementOf <{metric}> ;\n  \
                 dqv:computedOn <{batch}> ;\n  \
                 dqv:value {value:.4} ;\n  \
                 prov:wasAttributedTo <{agent}> ;\n  \
                 prov:generatedAtTime \"{ts}\"^^xsd:dateTime .",
                batch = batch_iri,
                slug = slug,
                metric = metric,
                value = value,
                agent = MACHINE_AGENT_IRI,
                ts = generated_at_time,
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(t)
    }
}

/// The canonical IRI naming one ingestion run, the subject the batch-quality
/// measurements are `dqv:computedOn`. `batch_id` is the caller's run identifier (a
/// fixture name, a pilot run id); it is used verbatim, so pass an IRI-safe token.
pub fn batch_iri(batch_id: &str) -> String {
    format!("{}/batch/{}", MACHINE_AGENT_IRI, batch_id)
}

/// The output of one pipeline run: the emitted Turtle (grounded machine-tier triples with
/// full PROV-O lineage) plus the [`Sidecar`].
#[derive(Debug, Clone)]
pub struct PipelineOutput {
    /// The emitted Turtle document (a complete, parseable graph: prefixes + the agent +
    /// the Sources + the grounded Findings + their extraction Activities). Conforms to
    /// `pkg.shapes.ttl` + `shapes/literature.shapes.ttl` by construction.
    pub turtle: String,
    /// The run sidecar (the Phase-5 metric + quarantine list).
    pub sidecar: Sidecar,
    /// The timestamp (UTC ISO 8601 xsd:dateTime) stamped on all emitted Findings and
    /// their extraction Activities via `prov:generatedAtTime`. In CI, this is injectable
    /// from the test fixtures; in live use, it defaults to the wall-clock instant.
    /// (sq-tzars.2 [HAIKU-4.5])
    pub generated_at_time: String,
}

/// The output of one TIERED pipeline run ([`run_tiered`], `sq-tzars.7`): the per-tier
/// Turtle ARTIFACTS (tier separation v1 = separate documents, not in-store named graphs).
///
/// The three payload artifacts partition every emitted machine-pipeline statement into
/// EXACTLY ONE tier by the source's captured licence ([`source_tier`], fail-closed). The
/// [`restricted_public_projection`](Self::restricted_public_projection) is a metadata-only
/// VIEW of the restricted tier — it is NOT a fourth tier and carries NO abstract-derived
/// text.
#[derive(Debug, Clone)]
pub struct TieredOutput {
    /// The MACHINE-tier artifact: full findings + PROV-O lineage for sources whose licence
    /// is known-redistributable. Stamped with the `pkg/graph#machine` IRI. Publishable.
    pub machine_tier: String,
    /// The LICENSE-RESTRICTED artifact: full findings for unknown/absent/non-redistributable
    /// sources. Stamped with `pkg/graph#license-restricted`. PRIVATE — never published as-is.
    pub license_restricted_tier: String,
    /// The metadata-only PUBLIC projection of the restricted tier: only the source IRI
    /// (DOI), title, year, and licence status. The load-bearing invariant is that it carries
    /// NO abstract-derived text (no `sigimpl:justification`, no `dcterms:abstract`, no
    /// `pkg:Finding`) — all a public dump may carry for a restricted source.
    pub restricted_public_projection: String,
    /// The run sidecar (grounding metric + quarantine list) — identical to a combined run.
    pub sidecar: Sidecar,
    /// The `prov:generatedAtTime` stamped on all emitted machine/restricted Findings.
    pub generated_at_time: String,
    /// Whether the connector batch was COMPLETE. When [`BatchCompleteness::CapTruncated`],
    /// EVERY emitted artifact carries an explicit incompleteness marker (a comment banner +
    /// an `rdfs:comment` on the tier-graph node) and MUST NOT be presented as the full set.
    pub completeness: BatchCompleteness,
}

/// Convert days-since-Unix-epoch (1970-01-01) to a Gregorian `(year, month, day)`.
///
/// The Fliegel–Van Flandern-style algorithm below expects a **Julian Day Number**, so
/// the epoch day is first shifted by `2_440_588` (the JDN of 1970-01-01). The original
/// sq-tzars.2 version omitted that shift and produced year `-4656` dates for the
/// current epoch — a bug CI never saw because every test injects a fixed timestamp;
/// caught + fixed by the sq-tzars.9 pilot, which keys its daily API-request ledger off
/// this clock. Direct unit tests below pin known dates incl. leap days. [FABLE-5]
pub(crate) fn civil_from_epoch_days(days: u64) -> (i64, i64, i64) {
    let a = days as i64 + 2_440_588 + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;
    (year, month, day)
}

/// Generate the current UTC time as an ISO 8601 xsd:dateTime string (e.g.,
/// `2026-07-05T14:30:00Z`). Used as the default `prov:generatedAtTime` when the caller
/// does not inject a fixed timestamp (as CI tests do for determinism).
/// (sq-tzars.2 [HAIKU-4.5]; shared with the pilot module for its `created_at` /
/// daily-ledger date — [FABLE-5] sq-tzars.9)
pub(crate) fn current_generated_at_time() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // Format as ISO 8601 UTC.
    let secs = now.as_secs();
    let nanos = now.subsec_nanos();
    // Days since UNIX epoch (1970-01-01).
    let days = secs / 86400;
    let secs_today = secs % 86400;
    let hours = secs_today / 3600;
    let mins = (secs_today % 3600) / 60;
    let secs_min = secs_today % 60;
    let (year, month, day) = civil_from_epoch_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}Z",
        year,
        month,
        day,
        hours,
        mins,
        secs_min,
        nanos / 1000
    )
}

/// Run the full pipeline over a recorded connector batch + a pluggable [`Extractor`]
/// (in CI, the replay [`super::extract::RecordedExtractor`]). Pure + deterministic; the
/// ONLY non-determinism a real run would have is inside the extractor, which is exactly
/// why it is a trait. NO network.
///
/// # Arguments
///
/// * `connector_json` - The OpenAlex batch JSON fixture (string)
/// * `extractor` - A trait object implementing [`Extractor`]
/// * `generated_at_time` - Optional override for the `prov:generatedAtTime` timestamp.
///   If `None`, uses the current UTC time. CI tests pass a fixed instant for determinism.
///   (sq-tzars.2 [HAIKU-4.5])
pub fn run_with_time<E: Extractor>(
    connector_json: &str,
    extractor: &E,
    generated_at_time: Option<String>,
) -> Result<PipelineOutput, String> {
    let prepared = prepare(connector_json, extractor)?;
    let gen_time = generated_at_time.unwrap_or_else(current_generated_at_time);
    // 4. emit the single combined machine-tier TTL (backward-compatible output).
    let turtle = emit_turtle(
        &prepared.stubs,
        &prepared.candidates,
        &prepared.grounded_by_doi,
        &gen_time,
    )?;
    Ok(PipelineOutput {
        turtle,
        sidecar: prepared.sidecar,
        generated_at_time: gen_time,
    })
}

/// Run the full pipeline with the default (current) timestamp.
/// Convenience wrapper around [`run_with_time`] for callers that do not need to inject
/// a fixed instant. (sq-tzars.2 [HAIKU-4.5])
pub fn run<E: Extractor>(connector_json: &str, extractor: &E) -> Result<PipelineOutput, String> {
    run_with_time(connector_json, extractor, None)
}

/// Run the full pipeline and partition the emission into the per-tier ARTIFACTS
/// (`sq-tzars.7`): a machine-tier document, a license-restricted document, and a
/// metadata-only public projection of the restricted tier. Every grounded finding is
/// routed to EXACTLY ONE tier by its source's captured licence ([`source_tier`],
/// fail-closed: unknown ⇒ restricted).
///
/// `completeness` records whether the input batch was the whole result set: pass
/// [`BatchCompleteness::from_pagination`] mapped from a paged connector run so a
/// cap-truncated batch is stamped with an explicit incompleteness marker in every artifact
/// (never presented as complete — the #1527 consumer note). For a single-shot fixture
/// batch the caller passes [`BatchCompleteness::Complete`].
pub fn run_tiered<E: Extractor>(
    connector_json: &str,
    extractor: &E,
    generated_at_time: Option<String>,
    completeness: BatchCompleteness,
) -> Result<TieredOutput, String> {
    let prepared = prepare(connector_json, extractor)?;
    let gen_time = generated_at_time.unwrap_or_else(current_generated_at_time);
    let machine_tier = emit_tier(&prepared, &gen_time, Tier::Machine, completeness)?;
    let license_restricted_tier =
        emit_tier(&prepared, &gen_time, Tier::LicenseRestricted, completeness)?;
    let restricted_public_projection = emit_restricted_projection(&prepared, completeness)?;
    Ok(TieredOutput {
        machine_tier,
        license_restricted_tier,
        restricted_public_projection,
        sidecar: prepared.sidecar,
        generated_at_time: gen_time,
        completeness,
    })
}

/// The connector→extract→ground middle, shared by [`run_with_time`] and [`run_tiered`] so
/// the grounding logic + sidecar have a single source of truth. `grounded_by_doi` maps a
/// source DOI to the INDICES (into `candidates`) of its grounded findings, in a stable
/// global order — the emitters number findings off this so a finding's IRI is identical
/// across the combined and per-tier outputs.
struct Prepared {
    stubs: Vec<SourceStub>,
    candidates: Vec<CandidateFinding>,
    sidecar: Sidecar,
    grounded_by_doi: HashMap<String, Vec<usize>>,
}

fn prepare<E: Extractor>(connector_json: &str, extractor: &E) -> Result<Prepared, String> {
    // 1. connector -> normalise.
    let (stubs, sources_skipped) = parse_openalex_batch(connector_json)?;
    // 2. extract (replay in CI).
    let candidates = extractor.extract(&stubs)?;
    // 3. ground (propose-then-verify).
    let (abstracts, source_dois) = ground::index(&stubs);

    let mut sidecar = Sidecar {
        candidates_total: candidates.len(),
        sources_skipped,
        ..Sidecar::default()
    };

    // Per source: collect the INDICES of grounded candidates so we can flip exploredStatus
    // and route each finding to its tier without cloning the candidates.
    let mut grounded_by_doi: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, c) in candidates.iter().enumerate() {
        match ground::verify(c, &abstracts, &source_dois) {
            Ok(()) => {
                sidecar.grounded += 1;
                grounded_by_doi.entry(c.source_doi.clone()).or_default().push(i);
            }
            Err(failure) => sidecar.quarantined.push(quarantine(c, &failure)),
        }
    }

    // exploredStatus per source: Explored iff it produced >=1 grounded Finding, else DeadEnd.
    for stub in &stubs {
        if grounded_by_doi.get(&stub.doi).is_some_and(|v| !v.is_empty()) {
            sidecar.sources_explored += 1;
        } else {
            sidecar.sources_dead_end += 1;
        }
    }

    Ok(Prepared {
        stubs,
        candidates,
        sidecar,
        grounded_by_doi,
    })
}

/// Record a quarantined candidate (truncating the justification for triage).
fn quarantine(c: &CandidateFinding, failure: &GroundingFailure) -> Quarantined {
    const MAX: usize = 80;
    let just = if c.justification.chars().count() > MAX {
        let truncated: String = c.justification.chars().take(MAX).collect();
        format!("{}…", truncated)
    } else {
        c.justification.clone()
    };
    Quarantined {
        source_doi: c.source_doi.clone(),
        justification: just,
        reason: failure.reason(),
    }
}

/// Escape a string for a Turtle `"…"` long-string-safe short literal: backslash, quote,
/// newline, CR, tab. (We emit short `"…"` literals; a justification is one line.)
fn ttl_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// Clamp the machine-tier confidence to the ceiling and force the assurance to
/// `secx:Conjectured` (the machine tier never outranks a human, §4.3). The emitted TTL is
/// therefore conformant to the literature shapes BY CONSTRUCTION; the shapes are the
/// durable backstop, not the only line of defence.
fn machine_tier(confidence: f64) -> f64 {
    confidence.min(MACHINE_CONFIDENCE_CEILING)
}

/// The Turtle prefix block shared by every emitted artifact (the `secx:` namespace is the
/// CANONICAL pkg one: w3id zkp-sparql sec-prop).
const TTL_PREFIXES: &str = "@prefix pkg:     <https://sparq.dev/ns/pkg#> .\n\
     @prefix prov:    <http://www.w3.org/ns/prov#> .\n\
     @prefix dcterms: <http://purl.org/dc/terms/> .\n\
     @prefix cito:    <http://purl.org/spar/cito/> .\n\
     @prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .\n\
     @prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .\n\
     @prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .\n\
     @prefix xsd:     <http://www.w3.org/2001/XMLSchema#> .\n\n";

/// The prefix block of the batch-quality artifact
/// ([`Sidecar::quality_measurements_turtle`]). Deliberately its OWN block, not an
/// extension of [`TTL_PREFIXES`]: the quality document is a separate artifact, and the
/// finding artifacts are byte-pinned by a golden test, so they must not gain a `dqv:`
/// prefix they never use.
const QUALITY_TTL_PREFIXES: &str = "@prefix prov: <http://www.w3.org/ns/prov#> .\n\
     @prefix dqv:  <http://www.w3.org/ns/dqv#> .\n\
     @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
     @prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .\n\n";

/// Write the extraction-agent node (typed `pkg:MachineAgent` so the literature shapes bind).
fn write_agent_node(t: &mut String) -> Result<(), String> {
    writeln!(
        t,
        "<{}> a pkg:MachineAgent ;\n  rdfs:label \"literature extraction agent (Phase-5 scaffolding, fixtures)\"@en .\n",
        MACHINE_AGENT_IRI
    )
    .map_err(|e| e.to_string())
}

/// Write one Source node (DOI-keyed, content-addressed): title, exploredStatus, and year
/// (when present). The single source of truth for the Source block, shared by the combined
/// and per-tier emitters so their output cannot drift.
fn write_source_node(t: &mut String, stub: &SourceStub, explored: bool) -> Result<(), String> {
    let src = stub.source_iri();
    let status = if explored {
        "pkg:Explored"
    } else {
        "pkg:DeadEnd"
    };
    write!(
        t,
        "<{src}> a pkg:Source ;\n  dcterms:title \"{title}\" ;\n  pkg:exploredStatus {status}",
        src = src,
        title = ttl_escape(&stub.title),
        status = status,
    )
    .map_err(|e| e.to_string())?;
    if let Some(y) = stub.year {
        write!(t, " ;\n  dcterms:issued \"{}\"^^xsd:gYear", y).map_err(|e| e.to_string())?;
    }
    t.push_str(" .\n");
    Ok(())
}

/// Write one grounded Finding node + its extraction Activity, with full PROV-O lineage and
/// the machine-tier caps (confidence ≤ ceiling, assurance forced to `secx:Conjectured`).
/// `finding_n` is the stable GLOBAL finding number, so a finding's IRI is identical whether
/// it lands in the combined output or a per-tier artifact. The single source of truth for
/// the Finding block. (Timestamps: sq-tzars.2 [HAIKU-4.5].)
fn write_finding_node(
    t: &mut String,
    stub: &SourceStub,
    c: &CandidateFinding,
    finding_n: usize,
    generated_at_time: &str,
) -> Result<(), String> {
    let src = stub.source_iri();
    let f = format!("{}/finding/{}", MACHINE_AGENT_IRI, finding_n);
    let act = format!("{}/activity/{}", MACHINE_AGENT_IRI, finding_n);
    let conf = machine_tier(c.confidence);
    writeln!(
        t,
        "<{f}> a pkg:Finding ;\n  \
         rdfs:label \"{verdict} (machine-extracted)\"@en ;\n  \
         sigimpl:justification \"{just}\" ;\n  \
         pkg:confidence {conf:.4} ;\n  \
         pkg:assurance secx:Conjectured ;\n  \
         prov:wasDerivedFrom <{src}> ;\n  \
         prov:wasGeneratedBy <{act}> ;\n  \
         prov:wasAttributedTo <{agent}> ;\n  \
         prov:generatedAtTime \"{generated_at_time}\"^^xsd:dateTime ;\n  \
         cito:citesAsEvidence <{src}> .\n\
         <{act}> a prov:Activity ;\n  \
         prov:used <{src}> ;\n  \
         prov:wasAssociatedWith <{agent}> ;\n  \
         prov:generatedAtTime \"{generated_at_time}\"^^xsd:dateTime .\n",
        f = f,
        verdict = ttl_escape(&c.verdict),
        just = ttl_escape(&c.justification),
        conf = conf,
        src = src,
        act = act,
        agent = MACHINE_AGENT_IRI,
        generated_at_time = generated_at_time,
    )
    .map_err(|e| e.to_string())
}

/// Emit the single COMBINED Turtle document: prefixes + the machine agent + every Source
/// (with its grounded Findings). Deterministic input-order; byte-stable. Backward-compatible
/// with the pre-tiering [`PipelineOutput::turtle`]. `grounded_by_doi` holds indices into
/// `candidates`. (Timestamps: sq-tzars.2 [HAIKU-4.5].)
fn emit_turtle(
    stubs: &[SourceStub],
    candidates: &[CandidateFinding],
    grounded_by_doi: &HashMap<String, Vec<usize>>,
    generated_at_time: &str,
) -> Result<String, String> {
    let mut t = String::new();
    t.push_str(TTL_PREFIXES);
    write_agent_node(&mut t)?;

    let mut finding_n = 0usize;
    for stub in stubs {
        let grounded = grounded_by_doi.get(&stub.doi);
        let explored = grounded.is_some_and(|v| !v.is_empty());
        write_source_node(&mut t, stub, explored)?;
        if let Some(idxs) = grounded {
            for &ci in idxs {
                finding_n += 1;
                write_finding_node(&mut t, stub, &candidates[ci], finding_n, generated_at_time)?;
            }
        }
    }
    Ok(t)
}

/// Build the header of a per-tier artifact: the shared prefixes, the extraction agent (when
/// the tier carries findings), and the self-identifying tier-graph node. When the source
/// batch was [`BatchCompleteness::CapTruncated`], the header ALSO carries an explicit
/// incompleteness marker — a comment banner AND an `rdfs:comment` on the tier-graph node —
/// so the artifact can never be mistaken for the full result set (`sq-tzars.7`).
fn tier_header(
    graph_iri: &str,
    tier_label: &str,
    with_agent: bool,
    completeness: BatchCompleteness,
) -> Result<String, String> {
    let mut t = String::new();
    if let BatchCompleteness::CapTruncated {
        pages_fetched,
        max_pages,
    } = completeness
    {
        writeln!(
            t,
            "# INCOMPLETE ARTIFACT -- the source batch was cut short by a pagination hard cap\n\
             # (pages_fetched={} == max_pages={}); MORE results may exist. This artifact is\n\
             # NOT the full result set and must not be presented as complete (sq-tzars.7).",
            pages_fetched, max_pages
        )
        .map_err(|e| e.to_string())?;
    }
    t.push_str(TTL_PREFIXES);
    if with_agent {
        write_agent_node(&mut t)?;
    }
    // The self-identifying tier-graph node (names which tier this artifact is).
    write!(
        t,
        "<{graph}> rdfs:label \"{label}\"@en",
        graph = graph_iri,
        label = ttl_escape(tier_label),
    )
    .map_err(|e| e.to_string())?;
    if let BatchCompleteness::CapTruncated {
        pages_fetched,
        max_pages,
    } = completeness
    {
        write!(
            t,
            " ;\n  rdfs:comment \"INCOMPLETE: cap-truncated batch (pages_fetched={} == max_pages={}); more results may exist -- this artifact is not the full result set.\"@en",
            pages_fetched, max_pages
        )
        .map_err(|e| e.to_string())?;
    }
    t.push_str(" .\n\n");
    Ok(t)
}

/// Compute the DOI-granularity effective tier for every normalised DOI across the full stub
/// batch (`sq-tzars.7` DOI-granularity fix). A DOI routes to [`Tier::Machine`] ONLY if EVERY
/// stub sharing that DOI carries an allowlisted licence — any absent, blank, or non-allowlisted
/// licence forces the WHOLE DOI to [`Tier::LicenseRestricted`] (fail-closed). This prevents a
/// dup-DOI batch with conflicting licence metadata from emitting the same abstract-derived
/// finding content into both tiers (violating the one-tier-per-statement invariant).
fn doi_tier_map(stubs: &[SourceStub]) -> HashMap<String, Tier> {
    let mut map: HashMap<String, Tier> = HashMap::new();
    for stub in stubs {
        let entry = map.entry(stub.doi.clone()).or_insert(Tier::Machine);
        // Once restricted, it stays restricted (fail-closed).
        if *entry == Tier::Machine && source_tier(stub.license.as_deref()) != Tier::Machine {
            *entry = Tier::LicenseRestricted;
        }
    }
    map
}

/// Emit ONE tier artifact ([`Tier::Machine`] or [`Tier::LicenseRestricted`]) — the full
/// Sources and Findings for the sources routed to that tier by [`source_tier`]. The global
/// `finding_n` counter is advanced for EVERY grounded finding (whether or not it is in this
/// tier), so a finding's IRI is stable across tiers and the machine/restricted artifacts
/// never share a Finding IRI: exactly one tier per emitted finding.
fn emit_tier(
    p: &Prepared,
    gen_time: &str,
    tier: Tier,
    completeness: BatchCompleteness,
) -> Result<String, String> {
    let label = match tier {
        Tier::Machine => {
            "machine tier -- literature-pipeline findings from redistributable-licensed sources"
        }
        Tier::LicenseRestricted => {
            "license-restricted tier -- machine findings from unknown/absent/non-redistributable sources (PRIVATE)"
        }
        Tier::HandAuthored => {
            return Err("the literature pipeline does not emit the hand-authored tier".to_string())
        }
    };
    let mut t = tier_header(tier.graph_iri(), label, true, completeness)?;

    // DOI-granularity routing (sq-tzars.7): compute the effective tier for each DOI across
    // ALL stubs before iterating — a dup-DOI batch with one cc-by stub and one licence-absent
    // stub routes the WHOLE DOI restricted (fail-closed). Then process each DOI exactly once
    // (first-occurrence representative) to keep `finding_n` stable across both tier runs.
    let doi_tiers = doi_tier_map(&p.stubs);
    let mut seen_dois: HashSet<&str> = HashSet::new();
    let mut finding_n = 0usize;
    for stub in &p.stubs {
        // Use the DOI-granularity effective tier, not the per-stub source_tier.
        let effective_tier = doi_tiers
            .get(&stub.doi)
            .copied()
            .unwrap_or(Tier::LicenseRestricted);
        let in_tier = effective_tier == tier;
        // Process each DOI exactly once: the first stub encountered is the representative
        // (subsequent stubs for the same DOI are skipped; finding_n stays consistent).
        if !seen_dois.insert(stub.doi.as_str()) {
            continue;
        }
        let grounded = p.grounded_by_doi.get(&stub.doi);
        if in_tier {
            let explored = grounded.is_some_and(|v| !v.is_empty());
            write_source_node(&mut t, stub, explored)?;
        }
        if let Some(idxs) = grounded {
            for &ci in idxs {
                finding_n += 1;
                if in_tier {
                    write_finding_node(&mut t, stub, &p.candidates[ci], finding_n, gen_time)?;
                }
            }
        }
    }
    Ok(t)
}

/// Emit the metadata-only PUBLIC projection of the license-restricted tier (`sq-tzars.7`):
/// for each restricted source, ONLY its IRI (DOI) + title + exploredStatus + year + licence
/// status. The load-bearing invariant — NO abstract-derived text: no `sigimpl:justification`,
/// no `dcterms:abstract`, and NO `pkg:Finding` at all. This is all a public dump may carry
/// for a restricted source. Truncation marker is propagated (an incomplete restricted batch
/// yields an incomplete projection).
fn emit_restricted_projection(
    p: &Prepared,
    completeness: BatchCompleteness,
) -> Result<String, String> {
    let mut t = tier_header(
        TIER_LICENSE_RESTRICTED_GRAPH,
        "license-restricted tier -- metadata-only PUBLIC projection (DOI + title + year + licence status; NO abstract-derived text)",
        false,
        completeness,
    )?;
    // DOI-granularity routing (sq-tzars.7): same effective-tier map used in emit_tier;
    // emit each restricted DOI's metadata exactly once (first-occurrence representative).
    let doi_tiers = doi_tier_map(&p.stubs);
    let mut seen_dois: HashSet<&str> = HashSet::new();
    for stub in &p.stubs {
        let effective_tier = doi_tiers
            .get(&stub.doi)
            .copied()
            .unwrap_or(Tier::LicenseRestricted);
        if effective_tier != Tier::LicenseRestricted {
            continue;
        }
        // Emit each DOI's metadata exactly once.
        if !seen_dois.insert(stub.doi.as_str()) {
            continue;
        }
        let src = stub.source_iri();
        let explored = p
            .grounded_by_doi
            .get(&stub.doi)
            .is_some_and(|v| !v.is_empty());
        let status = if explored {
            "pkg:Explored"
        } else {
            "pkg:DeadEnd"
        };
        write!(
            t,
            "<{src}> a pkg:Source ;\n  dcterms:title \"{title}\" ;\n  pkg:exploredStatus {status}",
            src = src,
            title = ttl_escape(&stub.title),
            status = status,
        )
        .map_err(|e| e.to_string())?;
        if let Some(y) = stub.year {
            write!(t, " ;\n  dcterms:issued \"{}\"^^xsd:gYear", y).map_err(|e| e.to_string())?;
        }
        write!(
            t,
            " ;\n  dcterms:license \"{}\"",
            ttl_escape(&license_status(stub.license.as_deref()))
        )
        .map_err(|e| e.to_string())?;
        t.push_str(" .\n");
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::literature::extract::RecordedExtractor;

    #[test]
    fn civil_from_epoch_days_matches_known_dates() {
        // Pins the JDN-offset fix ([FABLE-5] sq-tzars.9): the unshifted original mapped
        // the current epoch to year -4656. Expected values are independent calendar
        // facts (days since 1970-01-01), including leap days + a century leap year.
        assert_eq!(civil_from_epoch_days(0), (1970, 1, 1));
        assert_eq!(civil_from_epoch_days(20_640), (2026, 7, 6));
        assert_eq!(civil_from_epoch_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_epoch_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_epoch_days(18_321), (2020, 2, 29));
        assert_eq!(civil_from_epoch_days(19_419), (2023, 3, 3));
    }

    #[test]
    fn current_generated_at_time_is_iso8601_shaped() {
        // The live default (not injectable): shape + a sane current-era year.
        let t = current_generated_at_time();
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[7..8], "-");
        assert_eq!(&t[10..11], "T");
        assert!(t.ends_with('Z'));
        let year: i64 = t[..4].parse().unwrap();
        assert!((2026..2200).contains(&year), "sane wall-clock year, got {}", t);
    }

    fn run_fixture() -> PipelineOutput {
        let extractor = RecordedExtractor::from_fixture().unwrap();
        run(crate::literature::FIXTURE_OPENALEX_BATCH, &extractor).unwrap()
    }

    #[test]
    fn pipeline_grounds_some_and_quarantines_the_rest() {
        let out = run_fixture();
        // 6 candidates total; 3 are deliberately bad (fabricated span, Proven over-claim
        // grounds fine but stays Conjectured, dangling citation). The two bad-by-grounding
        // (fabricated span + dangling citation) are quarantined.
        assert_eq!(out.sidecar.candidates_total, 6);
        assert_eq!(out.sidecar.grounded + out.sidecar.quarantined.len(), 6);
        // Exactly 2 candidates fail grounding (the fabricated span + the dangling cite).
        assert_eq!(out.sidecar.quarantined.len(), 2);
        assert_eq!(out.sidecar.grounded, 4);
        // Nothing was silently dropped.
        assert!(out
            .sidecar
            .quarantined
            .iter()
            .any(|q| q.reason.contains("not a span")));
        assert!(out
            .sidecar
            .quarantined
            .iter()
            .any(|q| q.reason.contains("dangling") || q.reason.contains("does not resolve")));
    }

    #[test]
    fn grounding_rate_is_computed_not_hardcoded() {
        let out = run_fixture();
        let expected = out.sidecar.grounded as f64 / out.sidecar.candidates_total as f64;
        assert!((out.sidecar.grounding_rate() - expected).abs() < 1e-12);
    }

    #[test]
    fn emitted_turtle_caps_the_machine_tier() {
        let out = run_fixture();
        // The Proven over-claim (confidence 0.9) grounds fine but is emitted as
        // secx:Conjectured with confidence clamped to the 0.7 ceiling.
        assert!(out.turtle.contains("pkg:assurance secx:Conjectured"));
        assert!(!out.turtle.contains("secx:Proven"));
        assert!(out.turtle.contains("0.7000"));
        // Full PROV-O lineage is present.
        assert!(out.turtle.contains("prov:wasAttributedTo"));
        assert!(out.turtle.contains("prov:wasGeneratedBy"));
        assert!(out.turtle.contains("a pkg:MachineAgent"));
        // [HAIKU-4.5] sq-tzars.2: prov:generatedAtTime stamped on Findings + Activities.
        assert!(
            out.turtle.contains("prov:generatedAtTime"),
            "emitted Findings must carry prov:generatedAtTime for SHACL conformance"
        );
        assert!(
            out.turtle.contains("^^xsd:dateTime"),
            "prov:generatedAtTime must be typed as xsd:dateTime"
        );
    }

    #[test]
    fn explored_status_flips_per_source() {
        let out = run_fixture();
        // All three sources produced >=1 grounded finding, so all are Explored.
        assert_eq!(out.sidecar.sources_explored, 3);
        assert_eq!(out.sidecar.sources_dead_end, 0);
        assert!(out.turtle.contains("pkg:exploredStatus pkg:Explored"));
    }

    #[test]
    fn run_with_time_accepts_injected_timestamp_for_deterministic_tests() {
        // [HAIKU-4.5] sq-tzars.2: direct unit test for the new public `run_with_time`
        // function. Verify that an injected timestamp is used instead of the wall-clock
        // instant, enabling deterministic test fixtures.
        let extractor = RecordedExtractor::from_fixture().unwrap();
        let fixed_time = "2026-07-05T14:30:00Z";
        let out = run_with_time(crate::literature::FIXTURE_OPENALEX_BATCH, &extractor, Some(fixed_time.to_string())).unwrap();

        // The injected timestamp must appear in the emitted Turtle.
        assert_eq!(out.generated_at_time, fixed_time);
        assert!(out.turtle.contains(fixed_time), "injected timestamp must appear in emitted TTL");
        // Both Finding and Activity nodes must carry the timestamp.
        let timestamp_count = out.turtle.matches(fixed_time).count();
        assert!(
            timestamp_count >= 2,
            "both Findings and Activities should carry the timestamp (got {} occurrences)",
            timestamp_count
        );
    }

    // --- sq-tzars.7: tier partition ---------------------------------------

    #[test]
    fn source_tier_is_fail_closed_on_unknown_licence() {
        // Direct unit test for the public `source_tier`. Unknown/absent/blank/non-allowlist
        // licences route RESTRICTED (fail-closed); only allowlisted licences route MACHINE.
        assert_eq!(source_tier(None), Tier::LicenseRestricted);
        assert_eq!(source_tier(Some("")), Tier::LicenseRestricted);
        assert_eq!(source_tier(Some("   ")), Tier::LicenseRestricted);
        assert_eq!(source_tier(Some("cc-by-nc")), Tier::LicenseRestricted);
        assert_eq!(source_tier(Some("cc-by-nd")), Tier::LicenseRestricted);
        assert_eq!(source_tier(Some("all-rights-reserved")), Tier::LicenseRestricted);
        assert_eq!(source_tier(Some("cc-by")), Tier::Machine);
        assert_eq!(source_tier(Some("CC-BY")), Tier::Machine);
        assert_eq!(source_tier(Some("cc-by-sa")), Tier::Machine);
        assert_eq!(source_tier(Some("cc0")), Tier::Machine);
        assert_eq!(source_tier(Some("public-domain")), Tier::Machine);
    }

    #[test]
    fn tier_graph_iri_names_the_vocab_constant() {
        // Direct unit test for the public `Tier::graph_iri` (all three variants).
        assert_eq!(Tier::HandAuthored.graph_iri(), crate::vocab::TIER_HAND_AUTHORED_GRAPH);
        assert_eq!(Tier::Machine.graph_iri(), crate::vocab::TIER_MACHINE_GRAPH);
        assert_eq!(
            Tier::LicenseRestricted.graph_iri(),
            crate::vocab::TIER_LICENSE_RESTRICTED_GRAPH
        );
    }

    #[test]
    fn batch_completeness_from_pagination_detects_the_cap() {
        // Direct unit test for the public `BatchCompleteness::from_pagination` + `is_complete`.
        assert_eq!(
            BatchCompleteness::from_pagination(3, 20),
            BatchCompleteness::Complete
        );
        assert!(BatchCompleteness::from_pagination(3, 20).is_complete());
        // pages_fetched == max_pages ⇒ potentially truncated (fail-closed).
        let hit = BatchCompleteness::from_pagination(20, 20);
        assert_eq!(
            hit,
            BatchCompleteness::CapTruncated {
                pages_fetched: 20,
                max_pages: 20
            }
        );
        assert!(!hit.is_complete());
        // A zero cap is never treated as a truncation.
        assert!(BatchCompleteness::from_pagination(0, 0).is_complete());
    }

    #[test]
    fn run_tiered_routes_all_unknown_licence_findings_restricted() {
        // Direct unit test for the public `run_tiered`. The committed fixture carries NO
        // licence metadata, so every source is fail-closed to the restricted tier: the
        // machine tier gets ZERO findings and the restricted tier gets all four.
        let extractor = RecordedExtractor::from_fixture().unwrap();
        let out = run_tiered(
            crate::literature::FIXTURE_OPENALEX_BATCH,
            &extractor,
            Some("2026-07-05T14:30:00Z".to_string()),
            BatchCompleteness::Complete,
        )
        .unwrap();
        // Sidecar is identical to a combined run (4 grounded).
        assert_eq!(out.sidecar.grounded, 4);
        // Machine tier: no findings (all sources are restricted).
        assert!(!out.machine_tier.contains("a pkg:Finding"));
        // Restricted tier: all four grounded findings land here.
        assert_eq!(out.license_restricted_tier.matches("a pkg:Finding").count(), 4);
        // A Complete batch carries NO incompleteness marker.
        assert!(out.completeness.is_complete());
        assert!(!out.machine_tier.contains("INCOMPLETE"));
        assert!(!out.license_restricted_tier.contains("INCOMPLETE"));
    }

    // --- batch-quality DQV measurements (sq-2489d.5, design §4.5) -----------

    #[test]
    fn source_yield_rate_counts_explored_over_parsed_sources() {
        // Explicit arithmetic, independent of the fixture: 1 of 4 parsed sources yielded.
        let sc = Sidecar {
            sources_explored: 1,
            sources_dead_end: 3,
            // Skipped sources never reached extraction, so they must NOT move the yield.
            sources_skipped: 96,
            ..Sidecar::default()
        };
        assert!((sc.source_yield_rate() - 0.25).abs() < 1e-9);
        // Empty batch: vacuously 1.0 (mirrors grounding_rate) — nothing was wasted.
        assert_eq!(Sidecar::default().source_yield_rate(), 1.0);
        // All sources yielding is 1.0.
        let all = Sidecar {
            sources_explored: 3,
            ..Sidecar::default()
        };
        assert_eq!(all.source_yield_rate(), 1.0);
    }

    #[test]
    fn batch_iri_is_the_agent_scoped_run_iri() {
        assert_eq!(
            batch_iri("openalex-fixture"),
            format!("{}/batch/openalex-fixture", MACHINE_AGENT_IRI)
        );
    }

    #[test]
    fn quality_measurements_carry_both_metrics_with_the_computed_values() {
        let out = run_fixture();
        let iri = batch_iri("fixture");
        let ttl = out
            .sidecar
            .quality_measurements_turtle(&iri, "2026-07-05T14:30:00Z")
            .unwrap();
        // The batch node the measurements hang off, with its PROV lineage.
        assert!(ttl.contains(&format!("<{}> a prov:Activity", iri)));
        assert!(ttl.contains(&format!("prov:wasAssociatedWith <{}>", MACHINE_AGENT_IRI)));
        // Exactly the two metrics the pipeline computes — no unmeasured metric is emitted.
        assert_eq!(ttl.matches("a dqv:QualityMeasurement").count(), 2);
        assert!(ttl.contains(&format!(
            "dqv:isMeasurementOf <{}>",
            crate::vocab::GROUNDING_RATE_METRIC
        )));
        assert!(ttl.contains(&format!(
            "dqv:isMeasurementOf <{}>",
            crate::vocab::SOURCE_YIELD_RATE_METRIC
        )));
        // The VALUES are the sidecar's, not a constant: 4 of 6 candidates grounded.
        assert_eq!(out.sidecar.grounded, 4);
        assert_eq!(out.sidecar.candidates_total, 6);
        assert!(
            ttl.contains(&format!("dqv:value {:.4}", out.sidecar.grounding_rate())),
            "grounding-rate value must be the computed rate; got:\n{}",
            ttl
        );
        assert!(
            ttl.contains(&format!("dqv:value {:.4}", out.sidecar.source_yield_rate())),
            "source-yield value must be the computed rate; got:\n{}",
            ttl
        );
        // Every measurement is computedOn the batch and carries the injected timestamp.
        assert_eq!(ttl.matches(&format!("dqv:computedOn <{}>", iri)).count(), 2);
        assert_eq!(
            ttl.matches("\"2026-07-05T14:30:00Z\"^^xsd:dateTime").count(),
            3, // the batch activity + one per measurement
        );
    }

    #[test]
    fn quality_artifact_is_separate_from_the_finding_artifacts() {
        // The batch telemetry describes the RUN, not the graph content: it must not leak
        // findings, and the finding artifact must not gain the dqv: prefix (the golden
        // byte-for-byte artifact test depends on that).
        let out = run_fixture();
        let ttl = out
            .sidecar
            .quality_measurements_turtle(&batch_iri("fixture"), "2026-07-05T14:30:00Z")
            .unwrap();
        assert!(!ttl.contains("a pkg:Finding"));
        assert!(!ttl.contains("a pkg:Source"));
        assert!(!out.turtle.contains("dqv:"));
    }
}
