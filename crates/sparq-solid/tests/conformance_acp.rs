//! sq-3jtd.9 — the ACP **conformance corpus**: a table of independent,
//! minimal scenarios, each isolating one Solid ACP-spec construct, run through this
//! crate's ACP authorization engine (`materialize_acp` + `AuthIndex::accessible`) with
//! its expected `(agent, client, mode, resource) → allow | deny` decisions asserted.
//!
//! Decision: the conformance surface is the Solid ACP spec
//! (<https://solidproject.org/TR/acp>) at the library level — see the harness module
//! `src/conformance.rs` for the full rationale (why CTH-over-HTTP and the JS-reference
//! differential oracle are out-of-scope / research-open). The harness lets each
//! scenario fail independently and reports all mismatches at once.
//!
//! sq-t58w.6 — the scenario corpus itself now lives in the shared `tests/common`
//! module (`common::acp_corpus()`) so a SECOND test target (the differential oracle,
//! `sq-t58w.7`) can consume the IDENTICAL scenarios without copy-paste. This file is the
//! corpus's first consumer; the test set + assertions below are unchanged by the move.

mod common;

use common::{acp_corpus, ALICE, BOB, ACP_SCENARIO_FLOOR};
use sparq_solid::conformance::{run_corpus, AcpScenario, AcrBuilder, Decision, Expect};
use sparq_solid::Mode;

#[test]
fn acp_conformance_corpus_decision_parity() {
    let scenarios = acp_corpus();
    let reports = run_corpus(&scenarios).expect("every scenario materializes");

    let mut failures = String::new();
    let mut total_decisions = 0usize;
    let mut fail = 0usize;
    for r in &reports {
        total_decisions += r.checked();
        if !r.passed() {
            fail += 1;
            failures.push_str(&r.to_string());
        }
    }
    assert!(
        failures.is_empty(),
        "ACP conformance mismatch(es) across {} scenarios / {total_decisions} decisions:\n{failures}",
        reports.len(),
    );
    // sq-t58w.6 — the RATCHET line. The corpus move into `tests/common`
    // dropped the stdout summary the `solid-conformance` CI job greps; restore it in
    // the SHACL/geo runner shape (`pass N / fail M (floor F)`) so the belt-and-braces
    // grep gate re-checks the scenario count. `pass` is the count of passing scenarios
    // over the full corpus, so `>= floor` holds. KEEP this exact format — the CI grep
    // (`.github/workflows/ci.yml`) matches `ACP scenarios pass [0-9]+ / fail`.
    let pass = reports.len() - fail;
    println!("ACP scenarios pass {pass} / fail {fail} (floor {ACP_SCENARIO_FLOOR})");
    // Guard against a corpus that silently checks nothing.
    assert_eq!(reports.len(), ACP_SCENARIO_FLOOR, "expected 13 scenarios");
    assert!(
        pass >= ACP_SCENARIO_FLOOR,
        "ACP conformance scenario count regressed: {pass} < floor {ACP_SCENARIO_FLOOR}"
    );
    assert!(
        total_decisions >= 35,
        "expected a substantive decision table, got {total_decisions}"
    );
}

/// The harness reports a mismatch (rather than panicking) when an expected decision is
/// wrong — so a future regression surfaces as a readable diff, not a generic failure.
/// This NEGATIVE control deliberately states a wrong expectation and asserts the harness
/// flags it (it does not assert the engine is wrong — it asserts the *reporting* works).
#[test]
fn harness_flags_a_wrong_expectation() {
    let doc = "https://pod.example/neg/d1";
    let mut acr = AcrBuilder::new();
    acr.access_control(doc, |p| p.allow(Mode::Read).any_of_agent(ALICE));
    acr.document(doc);
    // bob has NO grant; asserting Allow is wrong on purpose.
    let scenario = AcpScenario::new("negative-control")
        .acr(acr)
        .expect(Expect::agent(BOB).read(doc).is(Decision::Allow));
    let report = scenario.run().expect("materializes");
    assert!(
        !report.passed(),
        "harness must detect the wrong expectation"
    );
    assert_eq!(report.mismatches().count(), 1);
    // the Display form names the mismatch
    assert!(report.to_string().contains("MISMATCH"));
}
