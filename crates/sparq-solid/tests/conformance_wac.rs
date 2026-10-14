//! sq-3jtd.8 — the WAC **conformance corpus**: a table of independent, minimal
//! scenarios, each isolating one Solid WAC-spec construct, run through this crate's WAC
//! authorization engine (`materialize_wac` + `AuthIndex::accessible`) with its expected
//! `(agent, client, mode, resource) → allow | deny` decisions asserted.
//!
//! Decision: the conformance surface is the Solid WAC spec
//! (<https://solidproject.org/TR/wac>) at the library level — see the harness module
//! `src/wac_conformance.rs` for the full rationale (why CTH-over-HTTP and the JS-reference
//! differential oracle are out-of-scope / research-open). This is the near-term-feasible
//! conformance signal called for in `research/sparq-solid-scope.md` §4. The harness lets
//! each scenario fail independently and reports all mismatches at once.
//!
//! The scenarios are derived from the WAC spec's normative semantics, NOT vendored over
//! HTTP from the live `solid/specification-tests` / CTH corpus — that corpus asserts on
//! HTTP-shaped outputs (status codes, the `WAC-Allow` header) which have no library entry
//! point here (they are PSS's by design, gh-55). The decision table is the authorization
//! *content* of those tests (the `.acl` shapes and their expected allow/deny per
//! principal), which is exactly what an authorization oracle can reproduce.
//!
//! sq-t58w.6 — the scenario corpus itself now lives in the shared `tests/common`
//! module (`common::wac_corpus()`) so a SECOND test target (the differential oracle,
//! `sq-t58w.7`) can consume the IDENTICAL scenarios without copy-paste. This file is the
//! corpus's first consumer; the test set + assertions below are unchanged by the move.

mod common;

use common::{wac_corpus, ALICE, BOB, WAC_SCENARIO_FLOOR};
use sparq_solid::conformance::{Decision, Expect};
use sparq_solid::wac_conformance::{run_corpus, AclBuilder, WacScenario};
use sparq_solid::Mode;

#[test]
fn wac_conformance_corpus_decision_parity() {
    let scenarios = wac_corpus();
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
        "WAC conformance mismatch(es) across {} scenarios / {total_decisions} decisions:\n{failures}",
        reports.len(),
    );
    // sq-t58w.6 — the RATCHET line. The corpus move into `tests/common`
    // dropped the stdout summary the `solid-conformance` CI job greps; restore it in
    // the SHACL/geo runner shape (`pass N / fail M (floor F)`) so the belt-and-braces
    // grep gate re-checks the scenario count. `pass` is the count of passing scenarios
    // over the full corpus, so `>= floor` holds. KEEP this exact format — the CI grep
    // (`.github/workflows/ci.yml`) matches `WAC scenarios pass [0-9]+ / fail`.
    let pass = reports.len() - fail;
    println!("WAC scenarios pass {pass} / fail {fail} (floor {WAC_SCENARIO_FLOOR})");
    // Guard against a corpus that silently checks nothing.
    assert_eq!(reports.len(), WAC_SCENARIO_FLOOR, "expected 13 scenarios");
    assert!(
        pass >= WAC_SCENARIO_FLOOR,
        "WAC conformance scenario count regressed: {pass} < floor {WAC_SCENARIO_FLOOR}"
    );
    assert!(
        total_decisions >= 40,
        "expected a substantive decision table, got {total_decisions}"
    );
}

/// The `PodStore` method form (the path a PSS integration uses, with the per-session cache)
/// reproduces the same decisions as the free-function `AuthIndex` path. Run the whole
/// corpus through `run_via_podstore` and assert parity.
#[test]
fn wac_conformance_via_podstore_matches() {
    for scenario in wac_corpus() {
        let report = scenario
            .run_via_podstore()
            .unwrap_or_else(|e| panic!("podstore run failed: {e}"));
        assert!(report.passed(), "{report}");
    }
}

/// The harness reports a mismatch (rather than panicking) when an expected decision is
/// wrong — so a future regression surfaces as a readable diff, not a generic failure. This
/// NEGATIVE control deliberately states a wrong expectation and asserts the harness flags it
/// (it does not assert the engine is wrong — it asserts the *reporting* works).
#[test]
fn harness_flags_a_wrong_expectation() {
    let doc = "https://pod.example/neg/d1";
    let mut acl = AclBuilder::new();
    acl.access_to(doc, |a| a.agent(ALICE).mode(Mode::Read));
    acl.document(doc);
    // bob has NO grant; asserting Allow is wrong on purpose.
    let scenario = WacScenario::new("negative-control")
        .acl(acl)
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
