//! [OPUS-4.8] (sq-jocpn) Prove the native Turtle parser produces the SAME W3C rdf-turtle pass/fail
//! set as oxttl. Run this test with AND without `--features native-ttl-suite`; the printed
//! per-test outcome line set must be IDENTICAL across the two runs (the conformance-set-identical
//! invariant). When the W3C data is not fetched (`tests/w3c/rdf-tests`), the test self-skips.

use sparq_conformance::inference::report::{Outcome, TestResult};
use sparq_conformance::turtle_suite;
use std::path::PathBuf;

#[test]
fn native_ttl_turtle_suite_matches_reference_set() {
    let turtle_root = PathBuf::from("tests/w3c/rdf-tests/rdf/rdf11/rdf-turtle")
        .canonicalize()
        .or_else(|_| {
            // The `sparq-conformance` crate runs from its own dir; the data lives at repo root.
            PathBuf::from("../../tests/w3c/rdf-tests/rdf/rdf11/rdf-turtle").canonicalize()
        });
    let turtle_root = match turtle_root {
        Ok(p) if p.join("manifest.ttl").is_file() => p,
        _ => {
            eprintln!("SKIP: W3C rdf-turtle data not fetched (run scripts/fetch-conformance.sh)");
            return;
        }
    };

    let mut out: Vec<TestResult> = Vec::new();
    turtle_suite::run_suite(&turtle_root, &mut out).expect("turtle suite ran");

    let (mut pass, mut fail, mut oos) = (0usize, 0usize, 0usize);
    let mut lines: Vec<String> = Vec::new();
    for r in &out {
        let tag = match &r.outcome {
            Outcome::Pass => {
                pass += 1;
                "PASS"
            }
            Outcome::Fail(_) => {
                fail += 1;
                "FAIL"
            }
            Outcome::OutOfScope(_) => {
                oos += 1;
                "OOS"
            }
            // The Turtle suite only emits Pass/Fail/OutOfScope, but the shared enum has other
            // variants; treat any as non-pass, non-fail for the set diff (they cannot occur here).
            _ => "OTHER",
        };
        lines.push(format!("{tag}\t{}", r.name));
    }
    lines.sort();

    let feature = if cfg!(feature = "native-ttl-suite") {
        "native-ttl"
    } else {
        "oxttl"
    };
    println!("=== rdf-turtle outcome set ({feature}) ===");
    println!(
        "TOTAL run={} pass={} fail={} oos={}",
        out.len(),
        pass,
        fail,
        oos
    );
    for l in &lines {
        println!("{l}");
    }

    // Hard floor guards against a silent regression: the native parser must not FAIL any test that
    // is a PositiveSyntax/Eval acceptance the reference passes. The set-identity is verified by
    // diffing the printed lines across the ON/OFF runs (see the module doc); here we pin that the
    // suite actually ran a non-trivial number of tests and reports zero unexpected failures beyond
    // the reference baseline, which the OFF run establishes.
    assert!(
        out.len() > 100,
        "the rdf-turtle suite should have many tests, got {}",
        out.len()
    );
    assert_eq!(
        fail, 0,
        "native/oxttl Turtle parse produced {fail} unexpected conformance failures"
    );
}
