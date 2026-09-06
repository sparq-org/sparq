//! [OPUS-4.8] sq-0wo9e.8 / P6 (epic sq-0wo9e, design `research/structure-aware-vectorisation.md`
//! §P6 eval harness) — the runnable **P0 ablation** for structure-aware vectorisation.
//!
//! Trains the thin in-tree shallow KGE ([`sparq_vectors::train`]) and measures it with the
//! standard FILTERED link-prediction harness ([`sparq_vectors::eval`]) across the 2×2 P0 prior
//! matrix `{closure on/off} × {type-constrained negatives, uniform-random negatives}`, with a
//! long-tail (head vs rare entity) breakdown, on two synthetic slices:
//!
//! - a WN18RR-style **structured relational** slice ([`synthetic_relational_ttl`]), and
//! - the synthetic **gUFO** slice ([`synthetic_gufo_ttl`]) (rigid kinds vs anti-rigid roles/phases).
//!
//! It runs the matrix under **both** scoring models — the symmetric **DistMult** and the asymmetric
//! **ComplEx** — because both slices are ~100 % directional, where DistMult is structurally
//! near-random (the adversarial-review finding). The ablation deltas must be read off the ASYMMETRIC
//! ComplEx run, and each cell is reported as a **mean ± std over several seeds** so a single-seed
//! delta is not mistaken for signal.
//!
//! **Firm-up (sq-4891y).** The headline closure-prior claim is not read off the *unpaired* cell
//! means (means ± stds, eyeballed). It is read off the **PAIRED** per-seed closure delta
//! ([`run_ablation_multiseed_paired`]): within each seed the four cells share the split / init /
//! negatives, so the paired difference cancels the shared noise and its spread is far smaller. The
//! run prints the paired closure delta as `mean ± se` with a significance flag at 1·se and 2·se, on
//! a **denser, schema-bearing** gUFO slice (the rigid `Person` kind is asserted on nobody, so the
//! RDFS closure must materialise it — the closure axis genuinely bites), over MANY seeds and more
//! epochs. It also runs an **LR sweep** so the maintainer can tune the step size on a canonical
//! machine. A prior is adopted only when the paired delta is significant on a schema-bearing KG
//! under ComplEx — never on these INDICATIVE work-box figures.
//!
//! ```sh
//! # Default: the synthetic slices, sized for this NON-CANONICAL work-box (runs in minutes).
//! cargo run -p sparq-vectors --release --features kge --example kge_ablation
//! cargo run -p sparq-vectors --release --features kge --example kge_ablation -- [entities] [people] [seed]
//!
//! # DATASET-GATED full run: point at a real N-Triples/Turtle KG (e.g. a WN18RR dump). Absent the
//! # env var the real-dataset arm is SKIPPED, so a casual run never needs a multi-GB download.
//! SPARQ_KGE_DATASET=/path/to/wn18rr.nt cargo run -p sparq-vectors --release --features kge --example kge_ablation
//! ```
//!
//! All numbers printed here are **INDICATIVE only** — the work-box is non-canonical, the model is a
//! tiny research-grade shallow KGE, and no figure is baked into any committed doc. The harness is the
//! instrument; the numbers must be re-measured on a canonical machine, on a REAL dataset, under the
//! asymmetric model, with multi-seed reporting, before any prior is adopted.

#[cfg(feature = "kge")]
fn main() {
    use sparq_vectors::eval::{
        run_ablation_multiseed, run_ablation_multiseed_paired, synthetic_gufo_ttl,
        synthetic_gufo_ttl_sized, synthetic_relational_ttl, EvalConfig, MultiSeedCell,
    };
    use sparq_vectors::ModelKind;

    let args: Vec<String> = std::env::args().collect();
    let entities: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(800);
    let people: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(600);
    let seed0: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20260620);

    // Five seeds: the mean ± std over them is what should be read, NOT a single run.
    let seeds: Vec<u64> = (0..5).map(|i| seed0.wrapping_add(i)).collect();

    println!("# sq-0wo9e P0 ablation — INDICATIVE numbers on a NON-CANONICAL work-box");
    println!("# filtered link-prediction: MRR / Hits@1 / Hits@3 / Hits@10 (higher is better)");
    println!("# matrix: closure {{off,on}} x negatives {{uniform,type-constrained}}");
    println!("# reported as MEAN +/- STD over {} seeds; read the ASYMMETRIC ComplEx run for any", seeds.len());
    println!("# inter-cell delta (DistMult is near-random on these 100%-directional slices).");
    println!();

    let print_cell = |c: &MultiSeedCell| {
        let label = format!(
            "closure={} neg={}",
            if c.closure { "ON " } else { "OFF" },
            if c.type_constrained { "type" } else { "unif" },
        );
        let m = &c.metrics;
        println!(
            "{:<24} q~{:<5.0} MRR={:.4}+/-{:.4} H@1={:.4} H@3={:.4} H@10={:.4}+/-{:.4}",
            label,
            m.queries.mean,
            m.mrr.mean,
            m.mrr.std,
            m.hits1.mean,
            m.hits3.mean,
            m.hits10.mean,
            m.hits10.std,
        );
        println!(
            "    head MRR={:.4}+/-{:.4} H@10={:.4}    tail MRR={:.4}+/-{:.4} H@10={:.4}",
            c.head.mrr.mean,
            c.head.mrr.std,
            c.head.hits10.mean,
            c.tail.mrr.mean,
            c.tail.mrr.std,
            c.tail.hits10.mean,
        );
    };

    let run_slice = |name: &str, ttl: &str| {
        // The work-box can afford a longer, slightly larger run than the test preset, for less
        // noisy indicative numbers (still NON-CANONICAL).
        for model in [ModelKind::DistMult, ModelKind::ComplEx] {
            let tag = match model {
                ModelKind::DistMult => "DistMult (SYMMETRIC — near-random on directional data)",
                ModelKind::ComplEx => "ComplEx (ASYMMETRIC — read deltas here)",
            };
            println!("== {} :: {} ==", name, tag);
            let mut cfg = EvalConfig::small(seed0);
            cfg.train.model = model;
            cfg.train.epochs = 150;
            cfg.train.dim = 64;
            cfg.train.negatives_per_positive = 16;
            match run_ablation_multiseed(ttl, "turtle", cfg, &seeds) {
                Ok(cells) => {
                    for c in &cells {
                        print_cell(c);
                    }
                }
                Err(e) => println!("  (ablation failed: {})", e),
            }
            println!();
        }
    };

    run_slice(
        "synthetic relational (WN18RR-style)",
        &synthetic_relational_ttl(entities, seed0),
    );
    run_slice(
        "synthetic gUFO slice",
        &synthetic_gufo_ttl(people, seed0 ^ 0x9),
    );

    // ---- Firm-up: PAIRED closure-delta on a denser schema-bearing gUFO slice (sq-4891y) --------
    // The headline closure claim, read off the variance-reduced PAIRED delta over MANY seeds, with
    // more epochs and a denser slice, under the asymmetric model. INDICATIVE only.
    {
        println!(
            "== FIRM-UP: paired closure delta on dense schema-bearing gUFO slice (ComplEx) =="
        );
        let n_seeds = 12usize;
        let seeds: Vec<u64> = (0..n_seeds as u64)
            .map(|i| seed0.wrapping_add(0x500 + i))
            .collect();
        let dense_ttl = synthetic_gufo_ttl_sized(people.max(400), 3, seed0 ^ 0x9);
        let mut cfg = EvalConfig::small(seed0);
        cfg.train.model = ModelKind::ComplEx;
        cfg.train.epochs = 250;
        cfg.train.dim = 64;
        cfg.train.negatives_per_positive = 16;
        match run_ablation_multiseed_paired(&dense_ttl, "turtle", cfg, &seeds) {
            Ok(r) => {
                let off = &r.cells[0].metrics.mrr; // C off, N unif (reference cell)
                let on = &r.cells[2].metrics.mrr; // C on,  N unif
                let unpaired = r.cells[0].metrics.mrr.std + r.cells[2].metrics.mrr.std;
                println!(
                    "  UNPAIRED view  : MRR(C off)={:.4}+/-{:.4}  MRR(C on)={:.4}+/-{:.4}  (sum-of-std={:.4})",
                    off.mean, off.std, on.mean, on.std, unpaired
                );
                let d = &r.closure_mrr;
                println!(
                    "  PAIRED closure : delta={:.4}  std={:.4}  se={:.4}  n={}  [sig@1se={}  sig@2se={}]",
                    d.mean,
                    d.std,
                    d.se,
                    d.n,
                    d.significant_at(1.0),
                    d.significant_at(2.0),
                );
                let dt = &r.closure_mrr_tail;
                println!(
                    "  PAIRED tail    : delta={:.4}  std={:.4}  se={:.4}  [sig@1se={}]",
                    dt.mean,
                    dt.std,
                    dt.se,
                    dt.significant_at(1.0),
                );
                let dn = &r.type_neg_mrr;
                println!(
                    "  PAIRED type-neg: delta={:.4}  std={:.4}  se={:.4}  [sig@1se={}]",
                    dn.mean,
                    dn.std,
                    dn.se,
                    dn.significant_at(1.0),
                );
                println!(
                    "  (read: a positive PAIRED delta clearing 2*se is the firm-up bar; the paired \
                     std should be << the unpaired sum-of-std.)"
                );
            }
            Err(e) => println!("  (paired ablation failed: {})", e),
        }
        println!();

        // LR sweep: the bead asks for an LR tune. Report the paired closure delta + se per LR so the
        // step size can be chosen on a canonical machine.
        println!("== FIRM-UP: LR sweep (paired closure delta per learning rate, ComplEx) ==");
        let lr_seeds: Vec<u64> = (0..6u64).map(|i| seed0.wrapping_add(0x900 + i)).collect();
        for &lr in &[0.03f32, 0.05, 0.1, 0.2, 0.3] {
            let mut cfg = EvalConfig::small(seed0);
            cfg.train.model = ModelKind::ComplEx;
            cfg.train.epochs = 200;
            cfg.train.dim = 64;
            cfg.train.negatives_per_positive = 16;
            cfg.train.lr = lr;
            match run_ablation_multiseed_paired(&dense_ttl, "turtle", cfg, &lr_seeds) {
                Ok(r) => {
                    let d = &r.closure_mrr;
                    println!(
                        "  lr={:<5} closure delta={:.4}+/-se{:.4}  [sig@2se={}]",
                        lr,
                        d.mean,
                        d.se,
                        d.significant_at(2.0)
                    );
                }
                Err(e) => println!("  lr={:<5} (failed: {})", lr, e),
            }
        }
        println!();
    }

    // ---- Phase-4 PROVENANCE-WEIGHTING: all three design-§USE-1 integration points --------------
    // [OPUS-5] sq-w2af4. The `w(t)` reader feeds three points, each with its OWN paired ablation:
    //   (1) the TRAINING axis  — `run_weight_ablation`  (sq-2489d.4): scale each positive's SGD step;
    //   (2) the POOLING  axis  — `run_pooling_ablation` (sq-w2af4):  structural-sketch pooling of
    //       a node's neighbours weighted by each ASSERTION's w(t), instead of the uniform mean. It
    //       can only move where the graph carries STATEMENT-level provenance (a reified statement);
    //       with entity-level provenance alone every edge of a subject shares its head weight and
    //       the delta is exactly zero by construction — a null SIGNAL, not a null result;
    //   (3) the FUSION   axis  — `ProvenanceWeights::weight_header` attaches the per-`Block`
    //       multiplier that `fuse_rrf_weighted` consumes (and `ground_weighted` rescales it per
    //       node from that node's own incident edges). It changes a *ranking fusion*, not a
    //       link-prediction score, so it has NO MRR delta to print here and is deliberately not
    //       faked into one; `tests/grounding.rs` proves that loop end-to-end instead.
    // Both printed deltas are INDICATIVE work-box numbers. The bar is pre-registered and the honest
    // verdict may be ABANDON — a non-positive or non-significant delta means the axis did not help
    // on this slice, and nothing here claims otherwise.
    {
        use sparq_vectors::eval::{run_pooling_ablation, run_weight_ablation, synthetic_provenance_ttl};

        println!("== PHASE-4 provenance-weighting: paired deltas on the annotated slice ==");
        let w_seeds: Vec<u64> = (0..8u64).map(|i| seed0.wrapping_add(0xB00 + i)).collect();
        let prov_ttl = synthetic_provenance_ttl(entities.max(200), seed0 ^ 0x2489);
        let mut cfg = EvalConfig::small(seed0);
        cfg.train.model = ModelKind::ComplEx;
        cfg.train.epochs = 150;
        cfg.train.dim = 64;
        cfg.train.negatives_per_positive = 16;

        match run_weight_ablation(&prov_ttl, "turtle", cfg, &w_seeds) {
            Ok(ab) => println!(
                "  (1) TRAINING axis: MRR delta={:.4}  se={:.4}  n={}  [adopt@2se={}]  \
                 H@10 delta={:.4}",
                ab.mrr.mean,
                ab.mrr.se,
                ab.mrr.n,
                ab.mrr_significant_at(2.0),
                ab.hits10.mean,
            ),
            Err(e) => println!("  (1) TRAINING axis: (failed: {})", e),
        }
        // The blend is a sweepable, NON-canonical knob — sweep it rather than trusting one value.
        for &blend in &[0.1f32, 0.25, 0.5] {
            match run_pooling_ablation(&prov_ttl, "turtle", cfg, &w_seeds, blend) {
                Ok(ab) => println!(
                    "  (2) POOLING  axis (blend={:<4}): MRR delta={:.4}  se={:.4}  n={}  \
                     [adopt@2se={}]  H@10 delta={:.4}",
                    blend,
                    ab.mrr.mean,
                    ab.mrr.se,
                    ab.mrr.n,
                    ab.mrr_significant_at(2.0),
                    ab.hits10.mean,
                ),
                Err(e) => println!("  (2) POOLING  axis (blend={}): (failed: {})", blend, e),
            }
        }
        println!(
            "  (3) FUSION   axis: no MRR delta by construction (it reweights RRF fusion, not \
             link-prediction) — see tests/grounding.rs."
        );
        println!(
            "  (read: a delta that does not clear 2*se is NO measured lift — the honest verdict \
             is ABANDON that axis on this slice.)"
        );
        println!();
    }

    // DATASET-GATED full run.
    if let Ok(path) = std::env::var("SPARQ_KGE_DATASET") {
        println!("== real dataset: {} ==", path);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let format = if path.ends_with(".ttl") {
                    "turtle"
                } else {
                    "ntriples"
                };
                let _ = format;
                run_slice(&format!("dataset {}", path), &text);
            }
            Err(e) => println!("  (could not read {}: {})", path, e),
        }
    } else {
        println!("# (real-dataset arm SKIPPED — set SPARQ_KGE_DATASET=/path/to/kg.nt to enable)");
    }
}

#[cfg(not(feature = "kge"))]
fn main() {
    eprintln!("kge_ablation requires --features kge");
}
