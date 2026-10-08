//! Data-gated smoke test for the inference runner: only runs when the suites
//! have been fetched (scripts/fetch-inference-suites.sh), so a clean checkout
//! stays green. The rdf-mt suite must be at 48/48 — it gates the harness
//! plumbing (manifest walking, closure, entailment check) end to end.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn inference_runner_produces_a_report_when_data_present() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c");
    if !root.join("rdf-tests/rdf/rdf11/rdf-mt").is_dir() || !root.join("n3/tests").is_dir() {
        eprintln!("skipping: inference test data absent (run scripts/fetch-inference-suites.sh)");
        return;
    }
    let report = std::env::temp_dir().join("sparq-inference-conformance-smoke.md");
    let status = Command::new(env!("CARGO_BIN_EXE_sparq-inference-conformance"))
        .arg("--root")
        .arg(&root)
        .arg("--report")
        .arg(&report)
        .status()
        .expect("run sparq-inference-conformance");
    assert!(status.success(), "runner must exit 0 without --strict");
    let text = std::fs::read_to_string(&report).expect("report written");
    assert!(
        text.contains("| rdf-mt | 48 | 0 |"),
        "rdf-mt stays at 48/48:\n{text}"
    );
    assert!(
        text.contains("Overall (all inference suites)"),
        "overall line present"
    );
}
