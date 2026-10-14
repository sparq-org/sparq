//! **Answer-qualification: hedge + abstention** (GenAI-KB Phase 2, `sq-2489d.2`, Use 2).
//!
//! This is the second consumer of the Phase-1 cross-cutting provenance join
//! ([`crate::provenance`]). Where [`crate::cite`] turns the join into *citations*, this
//! module turns the **same** resolved `pkg:assurance` / `pkg:confidence` axes into an
//! **answer-level qualification** — an honest hedge over the answer, and, below a
//! configurable floor, an **abstention**:
//!
//! - **`pkg:assurance` → a verb hedge.** All supporting bindings `secx:Proven` → an
//!   assertive verb; any `secx:Claimed` → "appears to be (claimed)"; any
//!   `secx:Conjectured`, or missing assurance, → "may be" (the weakest epistemic basis
//!   present dominates — the answer is only as strong as its weakest cited support).
//! - **`pkg:confidence` → a verbal band.** The *minimum* asserted confidence across the
//!   supporting bindings maps to High / Medium / Low against
//!   [`QualifyConfig`]-configured thresholds (again, weakest-link).
//! - **`min_confidence` → an abstention floor.** When the minimum supporting confidence
//!   is *below* the floor — or no supporting binding carries confidence at all and
//!   [`QualifyConfig::abstain_when_unprovenanced`] is set — the qualified answer
//!   **abstains** ("insufficient confidence to answer") rather than asserting.
//!
//! ## The load-bearing honesty invariants
//!
//! 1. **Weakest-link, never best-case.** The band and the verb hedge are driven by the
//!    *minimum* confidence and the *weakest* assurance present, so adding one
//!    low-confidence supporting binding can only *lower* the qualification, never raise
//!    it. (`research/provenance-driven-genai-kb.md` §5 Phase 2: "the verbal band tracks
//!    asserted `pkg:assurance` monotonically".)
//! 2. **Reflects asserted assurance — NOT "calibrated confidence".** The verbal band is a
//!    monotone reading of *hand-authored* `pkg:confidence` estimates; it is **not** a
//!    calibrated probability. No "calibrated confidence" is claimed here, and none should
//!    be, until a reliability-diagram measurement on calibrated confidences exists
//!    (design §5 Phase 2 / §0 honesty rule).
//! 3. **Fail-closed.** A subject with no provenance contributes the *weakest* assurance
//!    (`Missing`) and no confidence — it can only pull the answer toward abstention, never
//!    toward over-confidence. Over a no-provenance graph the honest default is to hedge
//!    maximally (and, with `abstain_when_unprovenanced`, to abstain).
//!
//! ## What this is NOT
//!
//! This does not *estimate* a confidence — it *reads* the symbolic `pkg:confidence` /
//! `pkg:assurance` the graph asserts (the design's "direct, symbolic evidence-uncertainty
//! signal", §5 Use 2). An answer over a graph that asserts no quality axes is hedged to
//! the weakest band — never silently asserted with invented confidence.

use crate::provenance::{self, ProvenanceRecord};
use sparq_core::Graph;
use sparq_engine::QueryResult;

/// Structural UNCALIBRATED disclaimer, always set on every [`AnswerQualification`].
///
/// `pkg:confidence` values in this crate are **hand-authored estimates** stored in the
/// knowledge-graph by a human author. No reliability-diagram measurement or calibration
/// harness yet validates them against observed frequencies (§6.4 rung C of the
/// calibration ladder builds that harness; this is rung B — making the fact structural
/// rather than a free-text footnote). Until rung C exists, no caller should interpret
/// the numeric confidence values as calibrated probabilities.
pub const UNCALIBRATED_NOTICE: &str =
    "confidence values are UNCALIBRATED hand-authored estimates — no reliability measurement exists yet";

/// The epistemic basis of a supporting binding, read from its `pkg:assurance` IRI's local
/// name (`secx:Proven` / `secx:Claimed` / `secx:Conjectured`). [`Assurance::Missing`] is
/// the fail-closed default for a binding that asserts no assurance — the **weakest** rung,
/// so it dominates any [`Assurance::weakest`] fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Assurance {
    /// No `pkg:assurance` asserted, or an unrecognised basis IRI — the weakest rung.
    Missing,
    /// `secx:Conjectured` — a conjecture / hypothesis.
    Conjectured,
    /// `secx:Claimed` — asserted but not proven.
    Claimed,
    /// `secx:Proven` — the strongest rung.
    Proven,
}

impl Assurance {
    /// Reads an assurance from a `pkg:assurance` IRI text, matching on the IRI's local
    /// name (`…#Proven` / `…/Claimed` / …). An absent (`None`) or unrecognised value is
    /// [`Assurance::Missing`] — fail-closed to the weakest rung. The match is
    /// case-insensitive on the local name only; the namespace is not constrained (the
    /// design references `secx:`, but any vocabulary using the same local names unifies).
    pub fn from_iri(iri: Option<&str>) -> Self {
        let Some(iri) = iri else {
            return Assurance::Missing;
        };
        // Local name = the text after the last `#` or `/`.
        let local = iri.rsplit(['#', '/']).next().unwrap_or(iri);
        match local.to_ascii_lowercase().as_str() {
            "proven" => Assurance::Proven,
            "claimed" => Assurance::Claimed,
            "conjectured" => Assurance::Conjectured,
            _ => Assurance::Missing,
        }
    }

    /// The **weakest** of two assurances — the load-bearing weakest-link fold. Because
    /// the variants are declared weakest-first and derive `Ord`, this is just `min`.
    pub fn weakest(self, other: Self) -> Self {
        self.min(other)
    }

    /// The verb hedge for this assurance rung, applied to an answer that *passes* the
    /// abstention floor. `Proven` asserts; `Claimed` hedges with "(claimed)"; everything
    /// weaker hedges with "may". Stable strings (a downstream verbaliser can match on
    /// them, or use [`Assurance`] directly).
    pub fn verb_hedge(self) -> &'static str {
        match self {
            Assurance::Proven => "is",
            Assurance::Claimed => "appears to be (claimed)",
            Assurance::Conjectured => "may be (conjectured)",
            Assurance::Missing => "may be (unverified)",
        }
    }
}

/// The verbal confidence band an answer's *minimum* supporting `pkg:confidence` falls
/// into. **Not** a calibrated probability — a monotone reading of asserted, hand-authored
/// confidence (see the module honesty contract). The bands are ordered weakest-first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfidenceBand {
    /// No supporting binding asserted a `pkg:confidence` — the weakest, fail-closed band.
    Unknown,
    /// Minimum supporting confidence below the low/medium threshold.
    Low,
    /// Minimum supporting confidence in `[low, high)`.
    Medium,
    /// Minimum supporting confidence at or above the medium/high threshold.
    High,
}

impl ConfidenceBand {
    /// A short human label (`"high"` / `"medium"` / `"low"` / `"unknown"`).
    pub fn label(self) -> &'static str {
        match self {
            ConfidenceBand::High => "high",
            ConfidenceBand::Medium => "medium",
            ConfidenceBand::Low => "low",
            ConfidenceBand::Unknown => "unknown",
        }
    }
}

/// Thresholds + floor controlling [`qualify_result`]. `Default` mirrors the `NlqConfig`
/// knob defaults: hedge ON once you opt into the `citations` feature, `min_confidence`
/// `None` (no abstention), and a 1/3–2/3 band split.
///
/// **Invariant on construction.** `min_confidence`, `low_band`, and `high_band` are read
/// as supplied; an out-of-order `low_band > high_band` does not panic but collapses the
/// `Medium` band to empty (every confidence is then `High` or `Low`) — documented rather
/// than enforced so the config stays a plain value type.
#[derive(Debug, Clone, PartialEq)]
pub struct QualifyConfig {
    /// Abstention floor: when the minimum supporting `pkg:confidence` is **strictly
    /// below** this value, the answer abstains. `None` (default) never abstains on
    /// confidence alone. Range is not enforced; a value `> 1.0` abstains on every
    /// confidence-bearing answer (a deliberate "always abstain unless Proven-strong"
    /// posture is then expressible via the thresholds).
    pub min_confidence: Option<f64>,
    /// Confidence `< low_band` → [`ConfidenceBand::Low`]. Default `1.0/3.0`.
    pub low_band: f64,
    /// Confidence `>= high_band` → [`ConfidenceBand::High`]. Default `2.0/3.0`.
    pub high_band: f64,
    /// When `true`, an answer whose supporting bindings carry **no** `pkg:confidence` at
    /// all abstains (treating "no asserted confidence" as below any floor). Default
    /// `false`: an un-provenanced answer is hedged to the weakest band but still returned
    /// (so the tool stays useful over graphs that simply do not carry the PKG axes).
    pub abstain_when_unprovenanced: bool,
}

impl Default for QualifyConfig {
    fn default() -> Self {
        Self {
            min_confidence: None,
            low_band: 1.0 / 3.0,
            high_band: 2.0 / 3.0,
            abstain_when_unprovenanced: false,
        }
    }
}

impl QualifyConfig {
    /// Maps a numeric confidence to its band against the configured thresholds.
    fn band_for(&self, conf: f64) -> ConfidenceBand {
        if conf >= self.high_band {
            ConfidenceBand::High
        } else if conf < self.low_band {
            ConfidenceBand::Low
        } else {
            ConfidenceBand::Medium
        }
    }
}

/// The qualification of one [`crate::Answer`]: the aggregate epistemic basis + band over
/// all supporting bindings, and whether the answer **abstains** under the floor.
///
/// Built by [`qualify_result`] from the Phase-1 provenance join — every field is a read of
/// asserted `pkg:assurance` / `pkg:confidence`, folded weakest-link. Reflects asserted
/// assurance; it is **not** a calibrated confidence.
///
/// ## Structural UNCALIBRATED disclaimer (§6.4 rung B)
///
/// The [`calibration_notice`](Self::calibration_notice) field is always
/// [`UNCALIBRATED_NOTICE`] — a structural (not free-text) reminder that the
/// `pkg:confidence` values driving `band` and `min_confidence` are hand-authored estimates
/// with no reliability measurement behind them. It is a field, not a doc comment, so any
/// code that reads or serialises this struct encounters it and cannot silently drop it.
#[derive(Debug, Clone, PartialEq)]
pub struct AnswerQualification {
    /// The weakest assurance present across all supporting bindings — drives the verb
    /// hedge. `Missing` when no binding asserts assurance (or there are no bindings).
    pub assurance: Assurance,
    /// The band of the *minimum* asserted confidence. [`ConfidenceBand::Unknown`] when no
    /// binding asserts `pkg:confidence`.
    pub band: ConfidenceBand,
    /// The minimum asserted `pkg:confidence` across supporting bindings (the value the
    /// band + floor are computed from). `None` when no binding asserts confidence.
    pub min_confidence: Option<f64>,
    /// `true` when the answer is **below the floor** (or unprovenanced under
    /// [`QualifyConfig::abstain_when_unprovenanced`]) — the caller should surface
    /// "insufficient confidence to answer" rather than the rows.
    pub abstain: bool,
    /// How many distinct supporting bindings were resolved (the subjects the answer rows
    /// bound). 0 means the result bound no IRIs — vacuously hedged to the weakest.
    pub supporting: usize,
    /// How many of the `supporting` bindings asserted **no** `pkg:confidence` — the
    /// un-provenanced fraction, kept explicit so a caller can report it honestly.
    pub unprovenanced: usize,
    /// **Always [`UNCALIBRATED_NOTICE`].** Structural disclaimer (§6.4 rung B): the
    /// `pkg:confidence` values that drive `band` and `min_confidence` are hand-authored
    /// estimates — no reliability-diagram measurement calibrates them. Present as a field
    /// (not a free-text phrase appendage) so serialisers and destructuring code see it and
    /// cannot silently drop it. Rung C of the calibration ladder builds the harness that
    /// will eventually justify retiring this notice.
    pub calibration_notice: &'static str,
}

impl AnswerQualification {
    /// A one-line honest qualification phrase for prefixing the answer. When the answer
    /// abstains this is the abstention sentence; otherwise it composes the verb hedge with
    /// the confidence band (e.g. "appears to be (claimed) [confidence: medium]"). Built
    /// only from asserted axes — never claims calibration.
    pub fn phrase(&self) -> String {
        if self.abstain {
            return "insufficient confidence to answer".to_string();
        }
        let band = match self.band {
            ConfidenceBand::Unknown => "unstated".to_string(),
            b => b.label().to_string(),
        };
        format!(
            "{} [confidence: {} (reflects asserted assurance, not calibrated)]",
            self.assurance.verb_hedge(),
            band
        )
    }
}

/// Parse a `pkg:confidence` lexical form to an `f64`. Tolerant: a non-numeric literal
/// (the graph should not assert one, but an adversarial graph might) is treated as **no
/// confidence** (`None`) rather than a panic — fail-closed toward hedging.
fn parse_confidence(lexical: Option<&str>) -> Option<f64> {
    lexical?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

/// Fold a single [`ProvenanceRecord`] into the running accumulators. A record with no
/// provenance still contributes `Assurance::Missing` (the weakest rung) so it pulls the
/// answer toward hedging. The assurance accumulator is an `Option` so that "no record yet"
/// is distinct from "a record with Missing assurance" — otherwise, starting at the weakest
/// rung, the weakest-link `min` could never climb (the bug a `Missing` seed would cause).
fn fold_record(
    rec: &ProvenanceRecord,
    weakest: &mut Option<Assurance>,
    min_conf: &mut Option<f64>,
    unprovenanced: &mut usize,
) {
    let a = Assurance::from_iri(rec.assurance.as_deref());
    *weakest = Some(match *weakest {
        Some(prev) => prev.weakest(a),
        None => a,
    });
    match parse_confidence(rec.confidence.as_deref()) {
        Some(c) => {
            *min_conf = Some(min_conf.map_or(c, |m| m.min(c)));
        }
        None => *unprovenanced += 1,
    }
}

/// Qualify a list of resolved [`ProvenanceRecord`]s (the output of
/// [`provenance::join`]) under `config`. This is the core, graph-free entry point — it
/// folds the records weakest-link and applies the abstention floor. [`qualify_result`]
/// is the convenience wrapper that runs the join first.
pub fn qualify_records(
    records: &[ProvenanceRecord],
    config: &QualifyConfig,
) -> AnswerQualification {
    let mut weakest: Option<Assurance> = None;
    let mut min_conf: Option<f64> = None;
    let mut unprovenanced = 0usize;

    // Fold every supporting binding. With zero records the answer is vacuously hedged to
    // the weakest (Missing / Unknown) — never silently asserted.
    for rec in records {
        fold_record(rec, &mut weakest, &mut min_conf, &mut unprovenanced);
    }
    // No record observed → vacuously weakest (Missing), never asserted.
    let weakest = weakest.unwrap_or(Assurance::Missing);

    let band = match min_conf {
        Some(c) => config.band_for(c),
        None => ConfidenceBand::Unknown,
    };

    // Abstention: below the floor, OR (when configured) no asserted confidence at all.
    let below_floor = match (config.min_confidence, min_conf) {
        (Some(floor), Some(c)) => c < floor,
        (Some(_), None) => config.abstain_when_unprovenanced,
        (None, _) => false,
    };
    let abstain = below_floor
        || (config.abstain_when_unprovenanced && min_conf.is_none() && !records.is_empty());

    AnswerQualification {
        assurance: weakest,
        band,
        min_confidence: min_conf,
        abstain,
        supporting: records.len(),
        unprovenanced,
        calibration_notice: UNCALIBRATED_NOTICE,
    }
}

/// The end-to-end entry point: run the cross-cutting provenance
/// [`join`](provenance::join) over a [`QueryResult`] and qualify it under `config`. This is
/// what [`crate::Answer::qualify`] calls. Read-only over `graph`.
///
/// **Projection contract.** The join (and so this fold) qualifies **every** `NamedNode`
/// bound in the result table — the answer-row bindings. A query that *also* projects a
/// supporting source IRI (which itself carries no `pkg:assurance`/`pkg:confidence`) folds
/// that source in as `Missing`/un-provenanced, pulling the qualification toward hedging —
/// the weakest-link guarantee. To qualify only the answer *subjects*, project just those
/// (e.g. `SELECT ?finding`, not `SELECT ?finding ?source`).
pub fn qualify_result(
    graph: &Graph,
    result: &QueryResult,
    config: &QualifyConfig,
) -> AnswerQualification {
    let records = provenance::join(graph, result);
    qualify_records(&records, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedNode;
    use sparq_engine::{query_with_budget, QueryBudget};

    // PKG-shaped fixture mirroring the real agents-findings.ttl Finding shape: two
    // sourced findings (Claimed/0.7 and Proven/0.9), one Proven/0.4 (low confidence),
    // and one with NO provenance at all. Drives the REAL provenance join, not a mock.
    const PKG: &str = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:      <http://ex/> .

ex:proven-high a pkg:Finding ;
  rdfs:label "MPC is semi-honest only" ;
  prov:wasDerivedFrom ex:source-mpc ;
  pkg:assurance secx:Proven ;
  pkg:confidence "0.9" .

ex:claimed-mid a pkg:Finding ;
  rdfs:label "ZK soundness internally re-audited" ;
  prov:wasDerivedFrom ex:source-tm ;
  pkg:assurance secx:Claimed ;
  pkg:confidence "0.5" .

ex:proven-low a pkg:Finding ;
  rdfs:label "a proven-but-low-confidence finding" ;
  prov:wasDerivedFrom ex:source-x ;
  pkg:assurance secx:Proven ;
  pkg:confidence "0.2" .

ex:bare a pkg:Finding ;
  rdfs:label "a finding with no recorded provenance" .
"#;

    fn graph() -> Graph {
        Graph::load_str(PKG, "turtle").unwrap()
    }

    fn nn(iri: &str) -> NamedNode {
        NamedNode::new(iri).unwrap()
    }

    fn select(g: &Graph, sparql: &str) -> QueryResult {
        query_with_budget(g, sparql, &QueryBudget::unlimited()).unwrap()
    }

    #[test]
    fn assurance_parses_local_name_and_defaults_missing() {
        assert_eq!(
            Assurance::from_iri(Some("https://w3id.org/zkp-sparql/sec-prop#Proven")),
            Assurance::Proven
        );
        assert_eq!(
            Assurance::from_iri(Some("http://x/Claimed")),
            Assurance::Claimed
        );
        assert_eq!(
            Assurance::from_iri(Some("http://x/Conjectured")),
            Assurance::Conjectured
        );
        // Unrecognised + absent both fail closed to Missing.
        assert_eq!(
            Assurance::from_iri(Some("http://x/Whatever")),
            Assurance::Missing
        );
        assert_eq!(Assurance::from_iri(None), Assurance::Missing);
    }

    #[test]
    fn assurance_weakest_link_orders_correctly() {
        // Missing < Conjectured < Claimed < Proven; weakest() == min().
        assert_eq!(
            Assurance::Proven.weakest(Assurance::Claimed),
            Assurance::Claimed
        );
        assert_eq!(
            Assurance::Proven.weakest(Assurance::Missing),
            Assurance::Missing
        );
        assert_eq!(
            Assurance::Claimed.weakest(Assurance::Conjectured),
            Assurance::Conjectured
        );
    }

    #[test]
    fn band_thresholds_default_split() {
        let cfg = QualifyConfig::default();
        assert_eq!(cfg.band_for(0.9), ConfidenceBand::High);
        assert_eq!(cfg.band_for(0.5), ConfidenceBand::Medium);
        assert_eq!(cfg.band_for(0.1), ConfidenceBand::Low);
        // Boundaries: >= high is High; < low is Low.
        assert_eq!(cfg.band_for(2.0 / 3.0), ConfidenceBand::High);
        assert_eq!(cfg.band_for(1.0 / 3.0), ConfidenceBand::Medium);
    }

    /// The DOCUMENTED degenerate-config invariant: an out-of-order `low_band > high_band`
    /// does not panic but COLLAPSES the `Medium` band to empty — every confidence is then
    /// `High` (>= high_band) or `Low` (< low_band), and `High` wins the `>=` check first.
    /// Asserts that no value maps to `Medium` under the inverted thresholds.
    #[test]
    fn band_for_inverted_thresholds_collapses_medium() {
        let cfg = QualifyConfig {
            low_band: 0.8,
            high_band: 0.2,
            ..QualifyConfig::default()
        };
        // 0.5 is >= high_band(0.2) → High (the `>=` branch is checked first).
        assert_eq!(cfg.band_for(0.5), ConfidenceBand::High);
        // 0.1 is < low_band(0.8) and < high_band → still hits the High `>=`? No: 0.1 < 0.2.
        assert_eq!(cfg.band_for(0.1), ConfidenceBand::Low);
        // Sweep a range: nothing lands in Medium under the inverted split.
        for i in 0..=10 {
            let c = i as f64 / 10.0;
            assert_ne!(
                cfg.band_for(c),
                ConfidenceBand::Medium,
                "inverted thresholds must collapse Medium, but {c} mapped to Medium"
            );
        }
    }

    /// `min_confidence` floor at the EXACT boundary: confidence == floor does NOT abstain
    /// (the floor is "strictly below"); a hair under it does. Locks the `< floor` predicate
    /// against an accidental `<=`.
    #[test]
    fn abstention_floor_is_strict_less_than() {
        let g = graph();
        let exactly = qualify_records(
            &[provenance::provenance_for(&g, &nn("http://ex/claimed-mid"))],
            &QualifyConfig {
                min_confidence: Some(0.5), // claimed-mid is exactly 0.5
                ..QualifyConfig::default()
            },
        );
        assert_eq!(exactly.min_confidence, Some(0.5));
        assert!(!exactly.abstain, "0.5 == floor must NOT abstain (strict <)");

        let under = qualify_records(
            &[provenance::provenance_for(&g, &nn("http://ex/claimed-mid"))],
            &QualifyConfig {
                min_confidence: Some(0.5001),
                ..QualifyConfig::default()
            },
        );
        assert!(under.abstain, "0.5 < 0.5001 floor must abstain");
    }

    #[test]
    fn weakest_link_min_confidence_drives_band() {
        // Answer over the Proven-high (0.9) AND Claimed-mid (0.5) findings: the WEAKEST
        // assurance (Claimed) and the MIN confidence (0.5 → Medium) must win.
        let g = graph();
        let r = select(
            &g,
            "PREFIX pkg: <https://sparq.dev/ns/pkg#> \
             PREFIX prov: <http://www.w3.org/ns/prov#> \
             SELECT ?f WHERE { VALUES ?f { <http://ex/proven-high> <http://ex/claimed-mid> } \
             ?f prov:wasDerivedFrom ?s } ORDER BY ?f",
        );
        let q = qualify_result(&g, &r, &QualifyConfig::default());
        assert_eq!(q.assurance, Assurance::Claimed, "weakest assurance wins");
        assert_eq!(
            q.band,
            ConfidenceBand::Medium,
            "min confidence 0.5 → medium"
        );
        assert_eq!(q.min_confidence, Some(0.5));
        assert!(!q.abstain);
        assert_eq!(q.supporting, 2);
        assert_eq!(q.unprovenanced, 0);
    }

    #[test]
    fn monotone_adding_a_lower_finding_only_lowers_the_band() {
        // RELIABILITY CHECK (design §5 Phase 2): the verbal band tracks asserted
        // assurance/confidence MONOTONICALLY — adding a weaker supporting binding can only
        // lower (never raise) the qualification.
        let g = graph();
        let cfg = QualifyConfig::default();

        let high_only = qualify_result(
            &g,
            &select(
                &g,
                "PREFIX prov: <http://www.w3.org/ns/prov#> \
                 SELECT ?f WHERE { VALUES ?f { <http://ex/proven-high> } ?f prov:wasDerivedFrom ?s }",
            ),
            &cfg,
        );
        let with_low = qualify_result(
            &g,
            &select(
                &g,
                "PREFIX prov: <http://www.w3.org/ns/prov#> \
                 SELECT ?f WHERE { VALUES ?f { <http://ex/proven-high> <http://ex/proven-low> } \
                 ?f prov:wasDerivedFrom ?s } ORDER BY ?f",
            ),
            &cfg,
        );
        // Both Proven (assurance unchanged), but the band must not increase.
        assert_eq!(high_only.assurance, Assurance::Proven);
        assert_eq!(with_low.assurance, Assurance::Proven);
        assert!(
            with_low.band <= high_only.band,
            "adding a lower-confidence finding raised the band: {:?} -> {:?}",
            high_only.band,
            with_low.band
        );
        assert_eq!(high_only.band, ConfidenceBand::High);
        assert_eq!(with_low.band, ConfidenceBand::Low); // min(0.9, 0.2) = 0.2 → Low
    }

    #[test]
    fn abstains_below_the_floor() {
        // min_confidence floor: an answer whose min supporting confidence (0.2) is below
        // the floor (0.5) abstains.
        let g = graph();
        let cfg = QualifyConfig {
            min_confidence: Some(0.5),
            ..QualifyConfig::default()
        };
        let r = select(
            &g,
            "PREFIX prov: <http://www.w3.org/ns/prov#> \
             SELECT ?f WHERE { VALUES ?f { <http://ex/proven-low> } ?f prov:wasDerivedFrom ?s }",
        );
        let q = qualify_result(&g, &r, &cfg);
        assert!(q.abstain, "0.2 < 0.5 floor must abstain");
        assert_eq!(q.phrase(), "insufficient confidence to answer");
    }

    #[test]
    fn does_not_abstain_at_or_above_the_floor() {
        let g = graph();
        let cfg = QualifyConfig {
            min_confidence: Some(0.5),
            ..QualifyConfig::default()
        };
        // proven-high (0.9) and claimed-mid (0.5): min 0.5 is NOT < 0.5 → no abstention.
        let r = select(
            &g,
            "PREFIX prov: <http://www.w3.org/ns/prov#> \
             SELECT ?f WHERE { VALUES ?f { <http://ex/proven-high> <http://ex/claimed-mid> } \
             ?f prov:wasDerivedFrom ?s }",
        );
        let q = qualify_result(&g, &r, &cfg);
        assert!(!q.abstain, "0.5 is at the floor, not below it");
    }

    #[test]
    fn unprovenanced_answer_hedges_weakest_and_can_abstain() {
        // FAIL-CLOSED: an answer over the no-provenance finding has Missing assurance,
        // Unknown band, and (with abstain_when_unprovenanced) abstains — never asserted.
        let g = graph();
        let r = select(
            &g,
            "PREFIX pkg: <https://sparq.dev/ns/pkg#> \
             SELECT ?f WHERE { VALUES ?f { <http://ex/bare> } ?f a pkg:Finding }",
        );
        // Default: hedged to weakest but NOT abstaining (the tool stays usable).
        let q = qualify_result(&g, &r, &QualifyConfig::default());
        assert_eq!(q.assurance, Assurance::Missing);
        assert_eq!(q.band, ConfidenceBand::Unknown);
        assert_eq!(q.min_confidence, None);
        assert!(!q.abstain);
        assert_eq!(q.unprovenanced, 1);
        assert!(q.phrase().contains("may be (unverified)"));

        // Opt into abstaining on no-confidence answers.
        let strict = QualifyConfig {
            abstain_when_unprovenanced: true,
            ..QualifyConfig::default()
        };
        let q2 = qualify_result(&g, &r, &strict);
        assert!(q2.abstain, "no asserted confidence + strict → abstain");
    }

    #[test]
    fn phrase_reflects_assurance_never_claims_calibration() {
        let g = graph();
        let r = select(
            &g,
            "PREFIX prov: <http://www.w3.org/ns/prov#> \
             SELECT ?f WHERE { VALUES ?f { <http://ex/claimed-mid> } ?f prov:wasDerivedFrom ?s }",
        );
        let q = qualify_result(&g, &r, &QualifyConfig::default());
        let p = q.phrase();
        assert!(p.contains("appears to be (claimed)"));
        // The honesty caveat is in the phrase: NOT calibrated.
        assert!(p.contains("not calibrated"));
        assert!(p.contains("medium"));
    }

    /// STRUCTURAL UNCALIBRATED DISCLAIMER (sq-2489d.9, §6.4 rung B): every
    /// [`AnswerQualification`] — regardless of its assurance, band, or abstention state —
    /// must carry [`UNCALIBRATED_NOTICE`] in its `calibration_notice` field. This is the
    /// coverage-ratchet unit test for the structural disclaimer (one direct test per new
    /// public field, per the brief). It is NOT enough that the phrase contains "not
    /// calibrated"; the field must be present and correct on the struct itself.
    #[test]
    fn qualified_answer_carries_structural_uncalibrated_notice() {
        let g = graph();
        // Test against a non-trivial qualified answer (Claimed/medium confidence).
        let r = select(
            &g,
            "PREFIX prov: <http://www.w3.org/ns/prov#> \
             SELECT ?f WHERE { VALUES ?f { <http://ex/claimed-mid> } ?f prov:wasDerivedFrom ?s }",
        );
        let q = qualify_result(&g, &r, &QualifyConfig::default());
        // The field must be exactly UNCALIBRATED_NOTICE (not a dynamic string, not empty).
        assert_eq!(
            q.calibration_notice,
            UNCALIBRATED_NOTICE,
            "calibration_notice must be UNCALIBRATED_NOTICE on every AnswerQualification"
        );
        assert!(
            q.calibration_notice.contains("UNCALIBRATED"),
            "notice must contain 'UNCALIBRATED'"
        );
        assert!(
            q.calibration_notice.contains("hand-authored"),
            "notice must contain 'hand-authored'"
        );
        // Also verify the empty-records (vacuous) case carries the notice — the disclaimer
        // must be present regardless of whether any supporting bindings were resolved.
        let empty = qualify_records(&[], &QualifyConfig::default());
        assert_eq!(
            empty.calibration_notice,
            UNCALIBRATED_NOTICE,
            "vacuous (empty-records) qualification must also carry UNCALIBRATED_NOTICE"
        );
    }

    #[test]
    fn empty_result_is_vacuously_weakest_not_asserted() {
        // An answer binding NO IRIs (e.g. an empty ASK-like row set) is hedged to the
        // weakest — never silently asserted with invented confidence.
        let g = graph();
        let r = select(
            &g,
            "SELECT ?f WHERE { VALUES ?f { <http://ex/does-not-exist> } ?f ?p ?o }",
        );
        assert_eq!(r.len(), 0);
        let q = qualify_result(&g, &r, &QualifyConfig::default());
        assert_eq!(q.assurance, Assurance::Missing);
        assert_eq!(q.band, ConfidenceBand::Unknown);
        assert_eq!(q.supporting, 0);
        assert!(!q.abstain);
    }

    #[test]
    fn non_numeric_confidence_is_treated_as_unprovenanced() {
        // An adversarial graph asserting a non-numeric pkg:confidence must not panic — it
        // is read as "no confidence" (fail-closed toward hedging).
        let g = Graph::load_str(
            r#"@prefix pkg: <https://sparq.dev/ns/pkg#> .
               @prefix prov: <http://www.w3.org/ns/prov#> .
               @prefix ex: <http://ex/> .
               ex:f a pkg:Finding ; prov:wasDerivedFrom ex:s ; pkg:confidence "not-a-number" .
               ex:s a pkg:Source ."#,
            "turtle",
        )
        .unwrap();
        let r = select(
            &g,
            "PREFIX prov: <http://www.w3.org/ns/prov#> \
             SELECT ?f WHERE { ?f prov:wasDerivedFrom ?s }",
        );
        let q = qualify_result(&g, &r, &QualifyConfig::default());
        assert_eq!(q.min_confidence, None);
        assert_eq!(q.band, ConfidenceBand::Unknown);
        assert_eq!(q.unprovenanced, 1);
        let _ = nn("http://ex/f"); // nn helper exercised
    }
}
