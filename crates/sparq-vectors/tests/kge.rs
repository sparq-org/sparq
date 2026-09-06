//! [OPUS-4.8] sq-0wo9e.8 / P6 (epic sq-0wo9e) — integration tests for the structure-aware
//! vectorisation MEASUREMENT FOUNDATION: the thin DistMult trainer + the filtered link-prediction
//! eval harness, exercising the REAL end-to-end path (parse → close → split → train → rank).
//!
//! These are the load-bearing invariants the design requires before any prior is adopted:
//! - the harness can detect NON-TRIVIAL ranking quality on a clearly-learnable relation (so a low
//!   number elsewhere is about the data/model, not a harness bug);
//! - the FILTERED protocol is correct (no train/test leakage; the filter is the union of all splits);
//! - the trainer actually learns; the ablation matrix runs every cell on the same split;
//! - the gUFO slice is a non-trivial baseline the harness can already ablate.
//!
//! No number is asserted as a quality claim — the only numeric assertion is a sanity FLOOR on a
//! deliberately-easy relation, to prove the instrument works.

#![cfg(feature = "kge")]

use sparq_reason::Profile;
use sparq_vectors::eval::{
    run_ablation, run_pooling_ablation, run_weight_ablation, synthetic_gufo_ttl,
    synthetic_provenance_ttl, synthetic_relational_ttl, EvalConfig, Splits,
};
use sparq_vectors::provenance::WeightMode;
use sparq_vectors::structure::close_for_vectorise;
use sparq_vectors::train::{train, TrainConfig};
use sparq_vectors::SamplingMode;
use std::collections::HashSet;

/// A clearly-learnable SYMMETRIC relation (DistMult's strength): K disjoint cliques connected by
/// `sim` within each clique. A correct harness MUST yield clearly non-trivial filtered MRR/Hits here.
fn clique_ttl(k: usize, per: usize) -> String {
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for c in 0..k {
        let members: Vec<usize> = (0..per).map(|m| c * per + m).collect();
        for &a in &members {
            for &b in &members {
                if a != b {
                    ttl.push_str(&format!("ex:e{} ex:sim ex:e{} .\n", a, b));
                }
            }
        }
    }
    ttl
}

#[test]
fn harness_detects_quality_on_learnable_relation() {
    // The instrument-validity test: on an easy symmetric relation the harness must report high
    // filtered metrics. If this regresses, the harness (not the data) is broken.
    let ttl = clique_ttl(12, 8);
    let mut cfg = EvalConfig::small(7);
    cfg.train.epochs = 120;
    cfg.train.dim = 48;
    cfg.train.negatives_per_positive = 16;
    let cells = run_ablation(&ttl, "turtle", cfg).unwrap();
    // No schema in this graph → all four cells must be identical (the priors are genuine no-ops).
    for c in &cells[1..] {
        assert!(
            (c.metrics.mrr - cells[0].metrics.mrr).abs() < 1e-9,
            "no-op priors must not change metrics"
        );
    }
    let m = &cells[0].metrics;
    assert!(m.queries > 0);
    // A deliberately conservative floor (the model reaches ~0.9 here; we assert well below that so
    // the test is robust, while still proving the harness measures real ranking quality).
    assert!(
        m.mrr > 0.5,
        "harness should detect strong ranking on a learnable relation: MRR={}",
        m.mrr
    );
    assert!(
        m.hits10 > 0.7,
        "Hits@10 floor on a learnable relation: {}",
        m.hits10
    );
}

#[test]
fn no_leakage_train_disjoint_from_test() {
    let ttl = synthetic_relational_ttl(300, 5);
    let c = close_for_vectorise(&ttl, "turtle", Profile::Rdfs).unwrap();
    let s = Splits::split(&c.graph, 0.8, 0.1, 42);
    let train: HashSet<[u32; 3]> = s.train.iter().copied().collect();
    let valid: HashSet<[u32; 3]> = s.valid.iter().copied().collect();
    let test: HashSet<[u32; 3]> = s.test.iter().copied().collect();
    assert!(
        train.is_disjoint(&test),
        "train must be disjoint from test (no leakage)"
    );
    assert!(
        train.is_disjoint(&valid),
        "train must be disjoint from valid"
    );
    assert!(valid.is_disjoint(&test), "valid must be disjoint from test");
    assert!(!test.is_empty(), "need a non-empty test split");
    // The filter set is the union of all three.
    assert_eq!(s.filter_set_len(), train.len() + valid.len() + test.len());
}

#[test]
fn trainer_learns_on_relational_slice() {
    let ttl = synthetic_relational_ttl(300, 9);
    let c = close_for_vectorise(&ttl, "turtle", Profile::Rdfs).unwrap();
    let tc = sparq_vectors::TypeConstraints::mine(&c.graph);
    let cfg = TrainConfig::small(SamplingMode::TypeConstrained, 1);
    let (model, report) = train(&c.graph, &tc, cfg);
    assert!(report.loss_decreased(), "loss must decrease");
    assert!(
        model.row_spread() > 1e-3,
        "embeddings must be non-degenerate"
    );
}

#[test]
fn ablation_matrix_runs_on_both_slices() {
    for (name, ttl) in [
        ("relational", synthetic_relational_ttl(200, 2)),
        ("gufo", synthetic_gufo_ttl(150, 3)),
    ] {
        let cfg = EvalConfig::small(11);
        let cells = run_ablation(&ttl, "turtle", cfg).unwrap_or_else(|e| panic!("{}: {}", name, e));
        assert_eq!(cells.len(), 4, "{}: 2x2 matrix", name);
        for cell in &cells {
            assert!(
                cell.metrics.queries > 0,
                "{}: every cell must produce scorable queries",
                name
            );
            assert!(
                cell.report.loss_decreased(),
                "{}: every cell's model must learn",
                name
            );
            // long-tail buckets partition the queries.
            assert_eq!(
                cell.long_tail.head.queries + cell.long_tail.tail.queries,
                cell.metrics.queries,
                "{}: long-tail buckets must partition queries",
                name
            );
            // The gUFO prior axis is exposed but OFF at the baseline phase.
            assert!(!cell.gufo_prior);
        }
    }
}

#[test]
fn gufo_closure_axis_actually_changes_something() {
    // The gUFO slice is designed so the RDFS closure is NOT a no-op (people are typed only by their
    // anti-rigid phase/role, so the rigid kind Person must be DERIVED). Confirm closure adds triples.
    let ttl = synthetic_gufo_ttl(150, 4);
    let c = close_for_vectorise(&ttl, "turtle", Profile::Rdfs).unwrap();
    assert!(
        c.entailed_triples > 0,
        "gUFO closure should derive entailed Person memberships (closure axis must bite)"
    );
}

// ---- Phase-4 provenance-weighting (sq-2489d.4) -------------------------------------------------

/// The load-bearing NO-OP invariant: over a provenance-FREE graph the ON arm is byte-identical to
/// OFF, so the paired MRR/Hits@10 delta is EXACTLY zero. (Provenance-weighting must never change a
/// plain graph — the design's "defaulting to 1.0 when no provenance exists" guarantee.)
#[test]
fn weight_ablation_is_exact_noop_on_provenance_free_graph() {
    // The relational slice carries no PROV-O/DQV annotations at all.
    let ttl = synthetic_relational_ttl(200, 3);
    let cfg = EvalConfig::small(13);
    let ab = run_weight_ablation(&ttl, "turtle", cfg, &[1, 2, 3]).unwrap();
    // Every per-seed arm is identical → the paired delta is bit-for-bit zero.
    assert_eq!(ab.mrr.mean, 0.0, "provenance-free MRR delta must be exactly 0");
    assert_eq!(ab.mrr.std, 0.0, "no spread when both arms are identical");
    assert_eq!(ab.hits10.mean, 0.0, "provenance-free Hits@10 delta must be exactly 0");
    // And the aggregated arm metrics agree exactly.
    assert_eq!(ab.off.mrr.mean, ab.on.mrr.mean, "arms must coincide on a plain graph");
}

/// The weight ablation runs END-TO-END on the provenance-annotated slice (parse → close → split →
/// train ON+OFF → rank), produces a well-formed paired delta over multiple seeds, and the firm-up
/// gate is well-defined. NO numeric lift is asserted — the bead's bar is "adopt only on measured
/// lift, ABANDON otherwise", and this is the instrument, not a claim.
#[test]
fn weight_ablation_runs_on_provenance_slice() {
    let ttl = synthetic_provenance_ttl(160, 5);
    let mut cfg = EvalConfig::small(21);
    cfg.train.epochs = 40;
    let seeds = [1u64, 2, 3, 4];
    let ab = run_weight_ablation(&ttl, "turtle", cfg, &seeds).unwrap();
    // Both arms scored real held-out queries.
    assert!(ab.off.mrr.n == seeds.len(), "one OFF sample per seed");
    assert!(ab.on.mrr.n == seeds.len(), "one ON sample per seed");
    assert!(ab.off.queries.mean > 0.0, "must score some held-out queries");
    // The paired delta is finite and the firm-up gate is callable (its verdict — adopt/abandon — is
    // the caller's policy; we only assert the instrument is well-formed, never a direction).
    assert!(ab.mrr.mean.is_finite() && ab.mrr.se.is_finite());
    let _adopt = ab.mrr_significant_at(2.0); // n>=2 → a real boolean, not trivially true
    assert_eq!(ab.mrr.n, seeds.len());
}

/// Proof the weight actually FLOWS into training: on the annotated slice, the
/// `WeightMode::Provenance` model's embeddings differ from the `WeightMode::Uniform` model's (the
/// down-weighted low-assurance positives take smaller gradient steps), so the ON arm is genuinely a
/// different model — not a silently-ignored flag. Over a provenance-free graph it would be identical
/// (covered by the no-op test above).
#[test]
fn provenance_weight_changes_the_trained_model() {
    let ttl = synthetic_provenance_ttl(140, 7);
    let c = close_for_vectorise(&ttl, "turtle", Profile::Rdfs).unwrap();
    let tc = sparq_vectors::TypeConstraints::mine(&c.graph);

    let mut off = TrainConfig::small(SamplingMode::TypeConstrained, 9);
    off.epochs = 30;
    off.weight_mode = WeightMode::Uniform;
    let mut on = off;
    on.weight_mode = WeightMode::Provenance;

    let (m_off, _) = train(&c.graph, &tc, off);
    let (m_on, _) = train(&c.graph, &tc, on);
    assert_eq!(m_off.dim, m_on.dim);
    // The two models must not be identical — the provenance weight reached the SGD step.
    let mut max_abs_diff = 0.0f32;
    for row in 0..m_off.num_entities() {
        for (a, b) in m_off.entity_vec(row).iter().zip(m_on.entity_vec(row).iter()) {
            max_abs_diff = max_abs_diff.max((a - b).abs());
        }
    }
    assert!(
        max_abs_diff > 1e-4,
        "provenance-weighting must change the trained embeddings (max|Δ|={max_abs_diff})"
    );
}

// ---- Phase-4 confidence-weighted POOLING ablation (sq-w2af4) -----------------------------------

/// [OPUS-5] The same load-bearing NO-OP invariant for the POOLING axis: over a provenance-free
/// graph the weighted pool is the uniform mean, so both arms produce identical embeddings and the
/// paired MRR/Hits@10 delta is EXACTLY zero. (Confidence-weighted pooling must never change a plain
/// graph — the design's "defaulting to 1.0 when no provenance exists" guarantee.)
#[test]
fn pooling_ablation_is_exact_noop_on_provenance_free_graph() {
    let ttl = synthetic_relational_ttl(200, 3);
    let cfg = EvalConfig::small(13);
    let ab = run_pooling_ablation(&ttl, "turtle", cfg, &[1, 2, 3], 0.25).unwrap();
    assert_eq!(ab.mrr.mean, 0.0, "provenance-free MRR delta must be exactly 0");
    assert_eq!(ab.mrr.std, 0.0, "no spread when both arms are identical");
    assert_eq!(ab.hits10.mean, 0.0, "provenance-free Hits@10 delta must be exactly 0");
    assert_eq!(ab.off.mrr.mean, ab.on.mrr.mean, "arms must coincide on a plain graph");
}

/// [OPUS-5] The pooling ablation runs END-TO-END on the provenance-annotated slice (parse → close →
/// split → train once → pool OFF+ON → rank), producing a well-formed paired delta over multiple
/// seeds. NO numeric lift is asserted — this is the instrument, not a claim; the honest verdict may
/// be "no measured lift, abandon".
#[test]
fn pooling_ablation_runs_on_provenance_slice() {
    let ttl = synthetic_provenance_ttl(160, 5);
    let mut cfg = EvalConfig::small(21);
    cfg.train.epochs = 40;
    let seeds = [1u64, 2, 3, 4];
    let ab = run_pooling_ablation(&ttl, "turtle", cfg, &seeds, 0.25).unwrap();
    assert_eq!(ab.off.mrr.n, seeds.len(), "one OFF sample per seed");
    assert_eq!(ab.on.mrr.n, seeds.len(), "one ON sample per seed");
    assert!(ab.off.queries.mean > 0.0, "must score some held-out queries");
    assert!(ab.mrr.mean.is_finite() && ab.mrr.se.is_finite());
    let _adopt = ab.mrr_significant_at(2.0); // n>=2 → a real boolean
    assert_eq!(ab.mrr.n, seeds.len());
}

/// [OPUS-5] Proof the pooling weights actually BITE: on the annotated slice the two arms score
/// differently (the weighted sketch is a genuinely different augmentation, not a silently-ignored
/// flag) — while a `blend` of `0.0` collapses both arms to the untouched model, so the delta is
/// exactly zero by construction. Together these show the ablation measures the pooling axis and
/// nothing else.
///
/// The bite comes specifically from the slice's **statement-level** provenance: the pool is keyed
/// by the asserting triple, so without reified statements every one of a subject's edges shares its
/// head weight and the two arms would be identical. That precondition is asserted rather than
/// assumed, so this test cannot quietly go vacuous.
#[test]
fn pooling_weights_change_the_scored_model_and_blend_zero_is_a_noop() {
    let ttl = synthetic_provenance_ttl(160, 11);
    let mined = sparq_vectors::provenance::ProvenanceWeights::mine(
        &sparq_core::Graph::load_str(&ttl, "turtle").unwrap(),
    );
    assert!(
        mined.annotated_statements() > 0,
        "precondition: the slice must carry per-statement provenance for this axis to bite"
    );
    let mut cfg = EvalConfig::small(5);
    cfg.train.epochs = 40;
    let seeds = [1u64, 2];

    let zero = run_pooling_ablation(&ttl, "turtle", cfg, &seeds, 0.0).unwrap();
    assert_eq!(zero.mrr.mean, 0.0, "blend=0 leaves both arms at the untouched model");

    let blended = run_pooling_ablation(&ttl, "turtle", cfg, &seeds, 0.5).unwrap();
    assert!(
        blended.off.mrr.mean != blended.on.mrr.mean || blended.mrr.std > 0.0,
        "confidence-weighted pooling must produce a different scored model on an annotated graph"
    );

    // A malformed blend is rejected, never silently coerced.
    assert!(run_pooling_ablation(&ttl, "turtle", cfg, &seeds, -1.0).is_err());
    assert!(run_pooling_ablation(&ttl, "turtle", cfg, &seeds, f32::NAN).is_err());
}
