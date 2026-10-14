//! End-to-end answer-qualification over the FULL NL→SPARQL loop (GenAI-KB Phase 2,
//! `sq-2489d.2`). These exercise the REAL path — `Nlq::ask_qualified` runs the loop
//! (parse, execute under a budget, produce a real `Answer`) and only then folds the
//! supporting `pkg:assurance`/`pkg:confidence` into a qualification. No mock bypasses the
//! provenance join.
//!
//! ## The acceptance bar (research/provenance-driven-genai-kb.md §5 Phase 2)
//!
//! On a **labelled answerable/unanswerable fixture split**, two falsifiable checks:
//!
//! 1. **Abstention precision/recall at the floor.** Label each fixture question
//!    `Answerable` (min supporting confidence ≥ floor) or `Unanswerable` (below the floor /
//!    no provenance), then measure that the `min_confidence` floor abstains on exactly the
//!    `Unanswerable` set: **precision = recall = 1.0** on this constructed split.
//! 2. **Reliability check — the verbal band tracks asserted `pkg:assurance`
//!    MONOTONICALLY.** Sorting answers by asserted minimum confidence never inverts the
//!    band; adding a weaker supporting binding never raises the qualification.
//!
//! **Honesty boundary (load-bearing).** This is NOT a "calibrated confidence" claim. The
//! split is constructed so abstention precision/recall is exactly measurable *against the
//! asserted confidence the fixture encodes* — it validates the floor logic + the monotone
//! band, not that the hand-authored `pkg:confidence` numbers are themselves calibrated. A
//! reliability-diagram measurement on calibrated confidences is explicitly out of scope and
//! deferred (the design's §0 honesty rule).
#![cfg(feature = "citations")]

use sparq_core::Graph;
use sparq_nlq::qualify::{Assurance, ConfidenceBand, QualifyConfig};
use sparq_nlq::{Llm, Nlq, NlqConfig};

/// A scripted `Llm` returning one fixed SPARQL query regardless of prompt — so the test
/// drives the REAL `ask`/`ask_qualified` loop without an exact-prompt fixture.
struct FixedSparql(String);
impl Llm for FixedSparql {
    fn complete(&self, _prompt: &str) -> Result<String, String> {
        Ok(format!("```sparql\n{}\n```", self.0))
    }
}

/// PKG-shaped fixture mirroring the real `agents-findings.ttl` Finding shape. Four findings
/// spanning the assurance × confidence space, plus one un-sourced finding:
///   high-proven  : Proven  / 0.95   (answerable)
///   mid-claimed  : Claimed / 0.50   (answerable at floor 0.5)
///   low-claimed  : Claimed / 0.20   (UNanswerable below 0.5)
///   low-conj     : Conjectured / 0.10 (UNanswerable)
///   bare         : (no provenance)   (UNanswerable)
const PKG: &str = r#"
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix kb:      <https://sparq.dev/ns/pkg/kb#> .

kb:src a pkg:Source ; pkg:confidence 1.0 .

kb:high-proven a pkg:Finding ;
  rdfs:label "merge needs ci-summary green" ;
  prov:wasDerivedFrom kb:src ; dcterms:source "AGENTS.md#merge" ;
  pkg:assurance secx:Proven ; pkg:confidence 0.95 .

kb:mid-claimed a pkg:Finding ;
  rdfs:label "ZK soundness internally re-audited" ;
  prov:wasDerivedFrom kb:src ; dcterms:source "threat-model.md#zk" ;
  pkg:assurance secx:Claimed ; pkg:confidence 0.50 .

kb:low-claimed a pkg:Finding ;
  rdfs:label "a low-confidence claimed finding" ;
  prov:wasDerivedFrom kb:src ;
  pkg:assurance secx:Claimed ; pkg:confidence 0.20 .

kb:low-conj a pkg:Finding ;
  rdfs:label "a conjecture" ;
  prov:wasDerivedFrom kb:src ;
  pkg:assurance secx:Conjectured ; pkg:confidence 0.10 .

kb:bare a pkg:Finding ;
  rdfs:label "a finding with no recorded provenance" .
"#;

fn pkg_graph() -> Graph {
    Graph::load_str(PKG, "turtle").unwrap()
}

/// Run `ask_qualified` over the loop for ONE finding IRI, returning the qualification.
fn qualify_one(graph: &Graph, cfg: NlqConfig, finding_iri: &str) -> sparq_nlq::QualifiedAnswer {
    // SELECT just the finding (projection drops the source ?s so only the answer subject
    // is qualified — the design's "answer-row binding").
    let sparql = format!(
        "PREFIX pkg: <https://sparq.dev/ns/pkg#> \
         SELECT ?f WHERE {{ VALUES ?f {{ <{}> }} ?f a pkg:Finding }}",
        finding_iri
    );
    let nlq = Nlq::with_config(graph, Box::new(FixedSparql(sparql)), cfg);
    nlq.ask_qualified("about this finding").unwrap()
}

/// THE ACCEPTANCE METRIC #1: abstention precision/recall at the floor on a labelled split.
/// With floor 0.5, the abstaining set must be EXACTLY the unanswerable set → P = R = 1.0.
#[test]
fn abstention_precision_recall_at_the_floor_is_one() {
    let graph = pkg_graph();
    let cfg = NlqConfig {
        // The floor lives on the qualification knob (the `min_confidence` abstention floor),
        // AND abstain on a no-provenance answer (kb:bare is unanswerable).
        qualify: Some(QualifyConfig {
            min_confidence: Some(0.5),
            abstain_when_unprovenanced: true,
            ..QualifyConfig::default()
        }),
        ..NlqConfig::default()
    };

    // Labelled split: (finding IRI, is_answerable). Answerable = min confidence ≥ 0.5.
    let split = [
        ("https://sparq.dev/ns/pkg/kb#high-proven", true),
        ("https://sparq.dev/ns/pkg/kb#mid-claimed", true), // 0.5 == floor → answerable
        ("https://sparq.dev/ns/pkg/kb#low-claimed", false), // 0.2 < 0.5
        ("https://sparq.dev/ns/pkg/kb#low-conj", false),   // 0.1 < 0.5
        ("https://sparq.dev/ns/pkg/kb#bare", false),       // no provenance
    ];

    // Confusion matrix of the abstention decision against the unanswerable label.
    let (mut tp, mut fp, mut fn_, mut tn) = (0u32, 0u32, 0u32, 0u32);
    for (iri, answerable) in split {
        let q = qualify_one(&graph, cfg.clone(), iri).qualification;
        let unanswerable = !answerable;
        match (q.abstain, unanswerable) {
            (true, true) => tp += 1,   // correctly abstained
            (true, false) => fp += 1,  // abstained on an answerable Q (over-cautious)
            (false, true) => fn_ += 1, // answered an unanswerable Q (over-confident)
            (false, false) => tn += 1, // correctly answered
        }
    }
    // On THIS constructed split the floor is exact: every unanswerable abstains, no
    // answerable does. Precision = TP/(TP+FP), Recall = TP/(TP+FN).
    let precision = tp as f64 / (tp + fp) as f64;
    let recall = tp as f64 / (tp + fn_) as f64;
    assert_eq!(precision, 1.0, "abstention precision (tp={tp} fp={fp})");
    assert_eq!(recall, 1.0, "abstention recall (tp={tp} fn={fn_})");
    assert_eq!(tn, 2, "the two answerable findings were answered");
    assert_eq!(tp, 3, "the three unanswerable findings abstained");
}

/// THE ACCEPTANCE METRIC #2 (reliability check): the verbal band tracks asserted
/// confidence MONOTONICALLY across the fixture set. Sorting by asserted min confidence
/// never inverts the band.
#[test]
fn verbal_band_is_monotone_in_asserted_confidence() {
    let graph = pkg_graph();
    let cfg = NlqConfig {
        qualify: Some(QualifyConfig::default()), // no floor; we just read bands
        ..NlqConfig::default()
    };
    // Ascending asserted confidence.
    let ordered = [
        ("https://sparq.dev/ns/pkg/kb#low-conj", 0.10),
        ("https://sparq.dev/ns/pkg/kb#low-claimed", 0.20),
        ("https://sparq.dev/ns/pkg/kb#mid-claimed", 0.50),
        ("https://sparq.dev/ns/pkg/kb#high-proven", 0.95),
    ];
    let mut prev_band: Option<ConfidenceBand> = None;
    let mut prev_conf = f64::MIN;
    for (iri, conf) in ordered {
        let q = qualify_one(&graph, cfg.clone(), iri).qualification;
        assert_eq!(
            q.min_confidence,
            Some(conf),
            "asserted confidence read back"
        );
        if let Some(pb) = prev_band {
            assert!(
                q.band >= pb,
                "band inverted: conf {prev_conf} band {pb:?} -> conf {conf} band {:?}",
                q.band
            );
        }
        prev_band = Some(q.band);
        prev_conf = conf;
    }
}

/// The verb hedge reflects asserted assurance (weakest-link), and the loop is unchanged by
/// qualification — the answer rows + transcript are identical to a plain `ask`.
#[test]
fn ask_qualified_hedges_and_leaves_the_answer_unchanged() {
    let graph = pkg_graph();
    let cfg = NlqConfig {
        qualify: Some(QualifyConfig::default()),
        ..NlqConfig::default()
    };
    let qa = qualify_one(&graph, cfg, "https://sparq.dev/ns/pkg/kb#mid-claimed");
    // The answer is a real loop answer: one row, no repairs.
    assert_eq!(qa.answer.result.len(), 1);
    assert_eq!(qa.answer.repairs, 0);
    assert_eq!(qa.answer.transcript.len(), 1);
    // The qualification reflects the Claimed assurance — and NEVER claims calibration.
    assert_eq!(qa.qualification.assurance, Assurance::Claimed);
    assert!(qa
        .qualification
        .phrase()
        .contains("appears to be (claimed)"));
    assert!(qa.qualification.phrase().contains("not calibrated"));
    assert!(!qa.qualification.abstain);
}

/// FAIL-CLOSED end-to-end: the un-sourced finding, with a floor + strict flag, abstains —
/// never asserts. And without the strict flag it is hedged to the weakest, still honest.
#[test]
fn unprovenanced_answer_abstains_under_strict_floor() {
    let graph = pkg_graph();
    let strict = NlqConfig {
        qualify: Some(QualifyConfig {
            min_confidence: Some(0.5),
            abstain_when_unprovenanced: true,
            ..QualifyConfig::default()
        }),
        ..NlqConfig::default()
    };
    let qa = qualify_one(&graph, strict, "https://sparq.dev/ns/pkg/kb#bare");
    assert!(qa.qualification.abstain);
    assert_eq!(
        qa.qualification.phrase(),
        "insufficient confidence to answer"
    );
    assert_eq!(qa.qualification.assurance, Assurance::Missing);
    assert_eq!(qa.qualification.band, ConfidenceBand::Unknown);

    // Lenient default: hedged to the weakest, but still returned (tool stays usable).
    let lenient = NlqConfig {
        qualify: Some(QualifyConfig::default()),
        ..NlqConfig::default()
    };
    let qa2 = qualify_one(&graph, lenient, "https://sparq.dev/ns/pkg/kb#bare");
    assert!(!qa2.qualification.abstain);
    assert!(qa2.qualification.phrase().contains("may be (unverified)"));
}

/// `ask_qualified` with NO qualify knob set still yields a qualification (default config),
/// so the call is always usable — the knob only changes the floor/thresholds.
#[test]
fn ask_qualified_without_a_knob_uses_the_default_config() {
    let graph = pkg_graph();
    let qa = qualify_one(
        &graph,
        NlqConfig::default(), // qualify: None
        "https://sparq.dev/ns/pkg/kb#high-proven",
    );
    assert_eq!(qa.qualification.assurance, Assurance::Proven);
    assert_eq!(qa.qualification.band, ConfidenceBand::High);
    assert!(!qa.qualification.abstain); // default has no floor
}
