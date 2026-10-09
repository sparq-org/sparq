//! [OPUS-4.8] sq-kuvu3 / [FABLE-5] sq-pbz04.3.4 (epic sq-pbz04) — the OWL 2 QL
//! (DL-Lite_R) query-rewriting arm of the `sparql11/entailment` conformance suite.
//!
//! ## What this lane is — and what it is deliberately NOT
//!
//! It runs the `sparq-reason-ql` query-rewriter over every `sd:EntailmentProfile
//! pr:QL` entailment test through the SIX-CONDITION graduation predicate
//! (sq-pbz04.3.4) and reports, HONESTLY, what came out: the sound subset reads
//! `QL graduated` (its enforcement is the SEPARATE pinned named-case ratchet in
//! `tests/ql_entailment_floor.rs`, a sparq-extension scoreboard row), and every
//! other row carries an exhaustive `QL experimental (<taxonomy>)` hold reason.
//!
//! In the inference BINARY this lane stays **NON-GATING for conformance**: every
//! row — graduated or held — is `Outcome::OutOfScope`, so no QL row can ever
//! count toward the binary's standards-conformance pass rate or ratchet floor
//! (the D-entailment precedent: graduation lives in the dedicated feature-gated
//! crate-test lane, never in the binary tally). That is the load-bearing design
//! choice this test asserts, ON the non-graduated remainder AND the graduated
//! subset alike:
//!   1. NO BINARY-RATCHET GRADUATION — every row the arm emits is OutOfScope.
//!   2. FAIL-CLOSED PRESERVED — a genuine held subset exists (the corpus is
//!      dominated by non-conjunctive / non-DL-Lite / intensional queries the
//!      rewriter must hold, never guess), and every held row carries a SPECIFIC
//!      taxonomy label — never the catch-all `unclassified-abstain` bucket.
//!   3. NO FAKED PASSES — a `QL graduated` row appears only when all six
//!      conditions genuinely passed (including empirical oracle
//!      result-equivalence); the set-equality enforcement is the floor test's.
//!
//! ## Feature gating (both states)
//!
//! The whole lane is behind this crate's opt-in `ql-experimental` feature
//! (forwards to `sparq-reason-ql/experimental`). With the feature OFF this file
//! compiles to a single self-SKIP `#[test]` (no QL rewriter links, and the
//! inference BINARY's QL section is `#[cfg]`-stripped, so its scoreboard is
//! byte-for-byte unchanged — the lean opt-in posture). The rdf-tests fixtures are
//! fetched by `scripts/fetch-inference-suites.sh` into the gitignored
//! `tests/w3c/rdf-tests/`; when absent the runner SKIPS so a fresh offline
//! checkout stays green.

#[cfg(not(feature = "ql-experimental"))]
#[test]
fn ql_experimental_arm_skipped_without_feature() {
    eprintln!(
        "SKIP: the OWL 2 QL query-rewriting arm is OFF — build with \
         `--features ql-experimental` (and run scripts/fetch-inference-suites.sh) to run it."
    );
}

#[cfg(feature = "ql-experimental")]
mod gated {
    use sparq_conformance::inference::report::Outcome;
    use sparq_conformance::inference::report::TestResult;
    use sparq_conformance::inference::sparql_entail;
    use std::path::PathBuf;

    fn rdf_tests_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdf-tests")
    }

    /// The exhaustive hold-taxonomy labels (mirrors `QlHoldReason::label`).
    /// `inconsistent-kb` added by sq-p6yb7 (a KB the DL-Lite_R consistency check
    /// proves inconsistent is held fail-closed, never an everything-entailed pass).
    const TAXONOMY: [&str; 10] = [
        "permanently-outside",
        "pending-gate",
        "pending-capture",
        "pending-consistency",
        "inconsistent-kb",
        "named-graph-dataset",
        "pending-coincidence",
        "oracle-divergent",
        "unclassified-abstain",
        "inconclusive",
    ];

    #[test]
    fn ql_arm_stays_out_of_scope_with_an_exhaustive_taxonomy() {
        let root = rdf_tests_root();
        if !root
            .join("sparql/sparql11/entailment/manifest.ttl")
            .exists()
        {
            eprintln!(
                "SKIP: rdf-tests `sparql11/entailment` not present under {} — run \
                 scripts/fetch-inference-suites.sh",
                root.display()
            );
            return;
        }

        let mut results: Vec<TestResult> = Vec::new();
        sparql_entail::run_ql_experimental_arm(&root, &mut results)
            .unwrap_or_else(|e| panic!("QL arm error: {e}"));

        assert!(
            !results.is_empty(),
            "the entailment suite has pr:QL-tagged tests — the arm must report them"
        );

        // INVARIANT 1 — NO BINARY-RATCHET GRADUATION: every row is OutOfScope in
        // the binary (never a Pass/Fail that counts to a rate or the binary's
        // ratchet floor). The pinned named-case floor lives in
        // tests/ql_entailment_floor.rs, tallied as a sparq-extension row.
        let mut graduated = 0usize;
        let mut held = 0usize;
        let mut unclassified: Vec<String> = Vec::new();
        for r in &results {
            let Outcome::OutOfScope(reason) = &r.outcome else {
                panic!(
                    "QL arm emitted a non-OutOfScope row for {:?} ({:?}) — that would \
                     risk a faked conformance pass in the binary ratchet",
                    r.name, r.outcome
                );
            };
            if reason.starts_with("QL graduated") {
                graduated += 1;
                // A graduated row must carry the honest extension-row disclaimer.
                assert!(
                    reason.contains("NEVER summed into the standards-conformance total"),
                    "graduated row without the extension-row disclaimer: {reason}"
                );
            } else if reason.starts_with("QL experimental (") {
                held += 1;
                // INVARIANT 2 — every held row carries a SPECIFIC taxonomy label.
                let label = reason
                    .trim_start_matches("QL experimental (")
                    .split(')')
                    .next()
                    .unwrap_or_default();
                assert!(
                    TAXONOMY.contains(&label),
                    "held row with a label outside the taxonomy ({label}): {reason}"
                );
                if label == "unclassified-abstain" {
                    unclassified.push(format!("{}: {}", r.name, reason));
                }
            } else {
                panic!("QL row with neither marker prefix: {reason}");
            }
        }

        // Positional args per the CodeQL guard in the shared agent contract.
        println!(
            "OWL 2 QL arm (sparql11/entailment pr:QL): {} tests — {} graduated \
             (pinned separately by ql_entailment_floor, a sparq-extension row), \
             {} held with taxonomy reasons (ALL rows OutOfScope in this binary)",
            results.len(),
            graduated,
            held
        );

        // INVARIANT 2 (continued) — FAIL-CLOSED PRESERVED: the corpus is dominated
        // by shapes the rewriter must hold, and no hold hides in the catch-all.
        assert!(
            held > 0,
            "the six-condition predicate must hold some QL-tagged queries (none held — \
             the predicate may have been weakened)"
        );
        assert!(
            unclassified.is_empty(),
            "unclassified-abstain rows present — extend the taxonomy explicitly: {:?}",
            unclassified
        );

        // INVARIANT 3 — NO FAKED PASSES: we do NOT assert a minimum graduated
        // count here (that is the floor test's set-equality job); the counts are
        // printed above for the record.
        let _ = graduated;
    }
}
