//! sq-qo1a9 (epic sq-pbz04, the LAST conformance bead) — the GRADUATED
//! OWL 2 QL (DL-Lite_R) certain-answer oracle ratchet.
//!
//! ## What this GRADUATES — and the honest scope
//!
//! The QL query-rewriting LOGIC (PerfectRef ∪ bounded tree-witness ∪
//! UCQ-containment minimisation) landed oracle-PROVEN in #1297 (sq-g19x0). It was
//! then wired into the `sparql11/entailment` arm as EXPERIMENTAL / OutOfScope in
//! #1312 (sq-kuvu3) — NO floor — because that broad `pr:QL` set is NOT the formal
//! OWL2-QL / DL-Lite_R suite: it mixes intensional / non-DL-Lite certain-answer
//! cases the sound query-rewriting fragment cannot answer (20 abstain / 10
//! computed-equivalent / 1 computed-DIVERGENT). That arm stays experimental
//! (`tests/ql_experimental_arm.rs` — untouched here).
//!
//! THIS lane graduates the FORMAL DL-Lite_R suite — the hand-derived oracle from
//! sq-g19x0 — to a pinned floor. Every case in `QL_DLLITE_ORACLE` is a conjunctive
//! query WITHIN sound DL-Lite_R rewriting, with a certain-answer set derived BY
//! HAND from the DL-Lite_R semantics (independently of the rewriter). The runner
//! rewrites each case with `rewrite_production`, evaluates the UCQ over the
//! UNMODIFIED ABox through the REAL engine, and asserts it returns EXACTLY the
//! certain answers — sound (no extra) AND complete (no missing) case by case. The
//! floor is the count of cases on which that holds.
//!
//! ## Why a sparq-EXTENSION row, NOT a standards-conformance number (the HONEST scope)
//!
//! There is no runnable normative W3C "OWL 2 QL certain-answer conformance suite"
//! to point a harness at — the W3C QL test material is structural/classification,
//! not an answer-comparison corpus over the query-rewriting semantics. So — exactly
//! like the RIF-Core expressivity / RSP-SRBench / BM25 oracles in this crate — this
//! is a faithful sparq-OWN oracle, registered as `family = "sparq extension"` and
//! tallied SEPARATELY in the central scoreboard, NEVER folded into the
//! standards-conformance total. It is HONESTLY a DL-Lite_R certain-answer oracle
//! floor, NOT a full-OWL-2-QL-conformance claim. The broad `pr:QL` entailment-arm
//! gap (the 1 intensional divergence) is documented + tracked, never summed in.
//!
//! ## Fail-closed + no faked passes
//!
//! Every case is rewritten through the REAL `rewrite_production` path (the
//! fail-closed CQ-shape gate runs first); a query it cannot soundly rewrite ABSTAINS
//! (reported, never a guessed pass), and the floor counts ONLY `SoundAndComplete`
//! outcomes. No case is special-cased — the certain answers are an independent
//! ground truth, so a rewriter regression that drops or invents an answer fails this
//! ratchet immediately.
//!
//! ## Feature gating (both states)
//!
//! The whole lane is behind this crate's opt-in `ql-experimental` feature (forwards
//! to `sparq-reason-ql/experimental`). With the feature OFF this file compiles to a
//! single self-SKIP `#[test]` (no QL rewriter links — the lean opt-in posture, so
//! the default + `--workspace` byte/bundle ratchets are byte-for-byte unchanged).
//! The corpus is IN-SOURCE (no fetched fixtures), so the lane runs on any checkout.
//! `QL_DLLITE_FLOOR` is read TEXTUALLY by `tests/scoreboard_floors.rs`, so the
//! central scoreboard's mirrored value can never silently drift.

// When the lane feature is OFF the runner is a single self-SKIP test so
// the default + `--workspace` builds neither link the QL rewriter nor go red. (cfg
// gate, not a runtime branch — zero QL code compiles in the default state, the
// lean-core invariant.)
#[cfg(not(feature = "ql-experimental"))]
#[test]
fn ql_dllite_suite_skipped_without_feature() {
    eprintln!(
        "SKIP: the GRADUATED OWL 2 QL (DL-Lite_R) certain-answer oracle lane is OFF — \
         build with `--features ql-experimental` to run it."
    );
}

#[cfg(feature = "ql-experimental")]
mod gated {
    use sparq_conformance::inference::sparql_entail::{
        run_ql_dllite_oracle, QlOracleOutcome, QL_DLLITE_ORACLE,
    };

    /// The OWL 2 QL (DL-Lite_R) certain-answer oracle FLOOR (the RATCHET). The
    /// MEASURED count of formal DL-Lite_R cases on which `rewrite_production` is
    /// sound AND complete (the rewritten UCQ, evaluated over the unmodified ABox,
    /// returns EXACTLY the hand-derived certain answers). MIRRORED in the central
    /// scoreboard (`scoreboard::SUITES`) and read TEXTUALLY by the guard test
    /// `tests/scoreboard_floors.rs`. It may only RISE — never lower it (raise as the
    /// DL-Lite_R oracle corpus grows). HONESTLY a sparq EXTENSION-shaped ratchet over
    /// the DL-Lite_R certain-answer oracle, NOT a full-OWL-2-QL-conformance claim.
    /// sq-qo1a9
    pub const QL_DLLITE_FLOOR: usize = 11;

    #[test]
    fn ql_dllite_certain_answer_ratchet() {
        let results = run_ql_dllite_oracle();

        let mut sound_complete = 0usize;
        let mut diverged: Vec<String> = Vec::new();
        let mut abstained: Vec<String> = Vec::new();
        let mut inconclusive: Vec<String> = Vec::new();
        for (id, outcome) in &results {
            match outcome {
                QlOracleOutcome::SoundAndComplete => sound_complete += 1,
                QlOracleOutcome::Diverged(why) => diverged.push(format!("{}: {}", id, why)),
                QlOracleOutcome::Abstained(why) => abstained.push(format!("{}: {}", id, why)),
                QlOracleOutcome::Inconclusive(why) => inconclusive.push(format!("{}: {}", id, why)),
            }
        }

        // The line a CI ratchet can grep (sound-and-complete count in
        // field $3), mirroring the RIF/RSP/BM25 EXTENSION ratchet shape. Positional
        // `println!` args per the CodeQL `rust/unused-variable` false-positive guard.
        println!("\nOWL 2 QL (DL-Lite_R) certain-answer oracle ratchet (sq-qo1a9)");
        println!(
            "QL DL-Lite_R sound-and-complete {} of {} (floor {})",
            sound_complete,
            QL_DLLITE_ORACLE.len(),
            QL_DLLITE_FLOOR
        );
        if !diverged.is_empty() {
            println!("DL-Lite_R DIVERGENCES (NOT summed into the floor):");
            for d in &diverged {
                println!("  - {}", d);
            }
        }
        if !abstained.is_empty() {
            println!("DL-Lite_R ABSTAINS (fail-closed, NOT summed into the floor):");
            for a in &abstained {
                println!("  - {}", a);
            }
        }
        if !inconclusive.is_empty() {
            println!("DL-Lite_R INCONCLUSIVE (setup, NOT summed into the floor):");
            for i in &inconclusive {
                println!("  - {}", i);
            }
        }

        // A formal-suite case must NEVER diverge: a divergence is a soundness OR a
        // completeness bug (the certain answers are an independent ground truth).
        assert!(
            diverged.is_empty(),
            "DL-Lite_R certain-answer oracle has DIVERGENT cases (a soundness/completeness \
             regression): {:?}",
            diverged
        );
        // Every formal case is constructed to be within sound rewriting, so an
        // abstain/inconclusive here means a fragment regression — never silently
        // tolerated.
        assert!(
            abstained.is_empty() && inconclusive.is_empty(),
            "DL-Lite_R oracle case unexpectedly abstained/inconclusive: abstains={:?} \
             inconclusive={:?}",
            abstained,
            inconclusive
        );
        assert!(
            sound_complete >= QL_DLLITE_FLOOR,
            "QL DL-Lite_R sound-and-complete count {} regressed below the ratchet floor {}",
            sound_complete,
            QL_DLLITE_FLOOR
        );
    }
}
