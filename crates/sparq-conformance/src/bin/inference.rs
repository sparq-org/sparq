//! sparq-inference-conformance: runs the reasoning test suites (RDF Semantics
//! rdf-mt, OWL 2 RL, N3, SPARQL entailment regimes) against `sparq-reason`
//! and emits the honest four-bucket scoreboard (stdout +
//! inference-conformance-report.md). Fetch the data once with
//! `scripts/fetch-inference-suites.sh`; everything runs offline after that.
//!
//! Informational by default (always exits 0); `--strict` exits 1 on any FAIL
//! so the pass rate can be ratcheted in CI once it stabilizes.
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

use sparq_conformance::inference::report::{Outcome, Section, TestResult};
use sparq_conformance::inference::{n3_suite, owl_suite, rdfmt, report, sparql_entail};
use sparq_conformance::turtle_suite;
use std::path::PathBuf;

fn main() {
    // Quiet expected panics (recorded as FAILs by the per-test watchdogs).
    std::panic::set_hook(Box::new(|_| {}));

    let mut root = PathBuf::from("tests/w3c");
    let mut report_path = PathBuf::from("inference-conformance-report.md");
    let mut strict = false;
    let mut filter: Option<String> = None;
    let mut verbose = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = PathBuf::from(args.next().expect("--root needs a path")),
            "--report" => report_path = PathBuf::from(args.next().expect("--report needs a path")),
            "--strict" => strict = true,
            "--filter" => filter = Some(args.next().expect("--filter needs a substring")),
            "--verbose" | "-v" => verbose = true,
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!(
                    "usage: sparq-inference-conformance [--root DIR] [--report FILE] [--filter SUBSTR] [--strict] [--verbose]"
                );
                std::process::exit(2);
            }
        }
    }

    let rdf_tests = root.join("rdf-tests");
    let n3_root = root.join("n3");
    let owl_export = root.join("owl2/all.rdf");
    // [OPUS-4.8] (B4) W3C rdf-turtle suite directory inside the same rdf-tests clone.
    let turtle_root = rdf_tests.join("rdf/rdf11/rdf-turtle");
    if !rdf_tests.is_dir() {
        eprintln!(
            "test data not found under {} — run scripts/fetch-inference-suites.sh first",
            root.display()
        );
        std::process::exit(2);
    }

    let mut sections: Vec<Section> = Vec::new();
    let mut run_section =
        |label: &str,
         notes: Vec<String>,
         f: &dyn Fn(&mut Vec<TestResult>) -> Result<(), String>| {
            let mut results = Vec::new();
            match f(&mut results) {
                Ok(()) => {}
                Err(e) => eprintln!("[{label}] suite error: {e}"),
            }
            if let Some(flt) = &filter {
                results.retain(|r| r.name.contains(flt.as_str()) || r.suite.contains(flt.as_str()));
            }
            if verbose {
                for r in &results {
                    let tag = match &r.outcome {
                        Outcome::Pass => "PASS".to_string(),
                        Outcome::Fail(e) => format!("FAIL ({e})"),
                        Outcome::Divergence(why, obs) => format!("DIVERGENCE ({why}; {obs})"),
                        Outcome::OutOfScope(why) => format!("OUT-OF-SCOPE ({why})"),
                    };
                    println!("[{}] {} — {}", r.suite, r.name, tag);
                }
            }
            sections.push(Section {
                label: label.to_string(),
                notes,
                results,
            });
        };

    run_section(
        "RDF Semantics entailment (rdf-tests rdf/rdf11/rdf-mt)",
        vec![
            "Premise → `sparq-reason` RDFS/RDF materialization (plus the harness-side finite \
             axiomatic/reflexive augmentation the production materializer deliberately omits) → \
             blank-node-homomorphism entailment check; `mf:result false` = (in)consistency check \
             with the per-test recognized-datatype set."
                .to_string(),
        ],
        &|results| rdfmt::run_suite(&rdf_tests, results),
    );
    run_section(
        "OWL 2 RL (W3C OWL WG test cases, RDF-based semantics)",
        owl_suite::notes(),
        &|results| owl_suite::run_suite(&owl_export, results),
    );
    run_section(
        "N3 (w3c/N3 community-group suite)",
        n3_suite::notes(),
        &|results| n3_suite::run_suite(&n3_root, results),
    );
    run_section(
        "SPARQL 1.1 entailment regimes (sparql11/entailment)",
        sparql_entail::notes(),
        &|results| sparql_entail::run_suite(&rdf_tests, results),
    );
    // [OPUS-4.8] sq-kuvu3 (epic sq-pbz04) — the EXPERIMENTAL OWL 2 QL (DL-Lite_R)
    // query-rewriting arm over the BROAD `pr:QL` `sparql11/entailment` set, opt-in
    // behind `ql-experimental`. Every row it adds is in the experimental
    // OUT-OF-SCOPE bucket (NOT a graduated conformance pass), so the section's
    // pass-rate is `—`/0 and the overall conformance totals are unchanged — and the
    // DEFAULT binary (feature off) is byte-for-byte unchanged (this section is
    // `#[cfg]`-stripped). It records, honestly, what `sparq-reason-ql` computes over
    // that broad set (which is NOT the formal DL-Lite_R suite — it mixes intensional
    // certain-answer cases outside sound rewriting). The FORMAL DL-Lite_R
    // certain-answer oracle GRADUATED to a pinned floor in sq-qo1a9 — it is enforced
    // by the `tests/ql_dllite_suite.rs` `--test` runner (a sparq-EXTENSION row in the
    // central scoreboard, `QL_DLLITE_FLOOR`), not summed into this binary's totals.
    #[cfg(feature = "ql-experimental")]
    run_section(
        "OWL 2 QL query-rewriting (EXPERIMENTAL, sparql11/entailment pr:QL — NOT the formal \
         DL-Lite_R suite, NOT a conformance floor; the formal DL-Lite_R oracle graduated in \
         sq-qo1a9 via tests/ql_dllite_suite.rs)",
        sparql_entail::ql_experimental_notes(),
        &|results| sparql_entail::run_ql_experimental_arm(&rdf_tests, results),
    );
    // [OPUS-4.8] (B4) W3C rdf-turtle suite through the SPARQ Turtle parser — the
    // rejection/acceptance oracle that gates the Turtle T1 spike. Reuses the rdf-tests clone.
    run_section(
        "W3C rdf-turtle through the sparq Turtle parser (parse_to_triples)",
        turtle_suite::notes(),
        &|results| turtle_suite::run_suite(&turtle_root, results),
    );

    let md = report::render(
        &sections,
        &[
            ("rdf-tests commit", rdf_tests.as_path()),
            ("w3c/N3 commit", n3_root.as_path()),
        ],
    );
    print!("{md}");
    if let Err(e) = std::fs::write(&report_path, &md) {
        eprintln!("could not write {}: {e}", report_path.display());
    } else {
        eprintln!("\nreport written to {}", report_path.display());
    }

    let total_fail: usize = sections
        .iter()
        .flat_map(|s| s.results.iter())
        .filter(|r| matches!(r.outcome, Outcome::Fail(_)))
        .count();
    if strict && total_fail > 0 {
        std::process::exit(1);
    }
}
