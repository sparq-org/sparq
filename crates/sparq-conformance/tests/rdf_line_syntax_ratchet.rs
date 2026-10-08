//! [FABLE-5] (sq-tonhr.2, epic sq-tonhr) The W3C **rdf-n-triples** / **rdf-n-quads** /
//! **rdf-trig** syntax-suite RATCHETS — the recorded floors that pin today's parser bar
//! BEFORE any rdf-shuttle generated candidate parser lands (floors may only RISE; a
//! candidate that changes any suite's outcome set is a regression the sq-tonhr.8/.9
//! differential gates must catch). Modelled on `native_ttl_ratchet.rs`: each run prints
//! the full per-test outcome line SET so two configurations can be diffed for outcome-SET
//! identity, not merely compared by count. When the W3C data is not fetched
//! (`tests/w3c/rdf-tests`, `scripts/fetch-conformance.sh`), the tests self-skip.
//!
//! The parse paths under test are the REAL ingest entry points (see
//! `sparq_conformance::line_syntax`): the native chunk-parallel `nt.rs` N-Triples parser,
//! the chunk-parallel N-Quads dataset loader, and the with-base TriG dataset loader.

use sparq_conformance::inference::report::{Outcome, TestResult};
use sparq_conformance::line_syntax::{run_suite, LineSuite};
use std::path::PathBuf;

/// rdf-n-triples floor: MEASURED pass count at the pinned w3c/rdf-tests revision —
/// 60 of the 70 manifest entries (41 positive + 29 negative syntax tests). The 10
/// honest FAILs are native `nt.rs` parser divergences RECORDED by wiring this suite
/// (tracked as bead sq-w64x5, to RAISE the floor when fixed): 9 LENIENT accepts of
/// negative cases (no IRI character validation: `bad-uri-01/04/06/07/08/09`; no
/// blank-node-label validation: `bad-bnode-01/02`; no language-tag validation:
/// `bad-lang-01`) and 1 STRICT reject of the positive `minimal_whitespace` case (a
/// `BLANK_NODE_LABEL` directly followed by `<` needs no whitespace per the grammar).
const NT_SYNTAX_FLOOR: usize = 60;
/// rdf-n-quads floor: MEASURED pass count at the pinned revision — 76 of the 87
/// manifest entries (53 positive + 34 negative syntax tests; the N-Quads manifest
/// embeds the N-Triples cases, N-Quads being a superset). The 11 honest FAILs are the
/// SAME native byte-level parser divergences as the N-Triples suite (shared `nt.rs`
/// fast path; bead sq-w64x5) plus `nq-syntax-bad-uri-01` (graph-position IRI
/// validation).
const NQ_SYNTAX_FLOOR: usize = 76;
/// rdf-trig floor: MEASURED pass count at the pinned revision (ALL 356 manifest
/// entries — 98 positive + 111 negative syntax, 143 eval, 4 negative eval — pass
/// through the with-base TriG dataset loader).
const TRIG_SYNTAX_FLOOR: usize = 356;

fn suite_root(dir: &str) -> Option<PathBuf> {
    let root = PathBuf::from(format!("tests/w3c/rdf-tests/rdf/rdf11/{dir}"))
        .canonicalize()
        .or_else(|_| {
            // The crate's tests run from its own dir; the data lives at the repo root.
            PathBuf::from(format!("../../tests/w3c/rdf-tests/rdf/rdf11/{dir}")).canonicalize()
        });
    match root {
        Ok(p) if p.join("manifest.ttl").is_file() => Some(p),
        _ => {
            eprintln!("SKIP: W3C {dir} data not fetched (run scripts/fetch-conformance.sh)");
            None
        }
    }
}

fn run_ratchet(suite: LineSuite, dir: &str, floor: usize) {
    let Some(root) = suite_root(dir) else { return };
    let mut out: Vec<TestResult> = Vec::new();
    run_suite(suite, &root, &mut out).expect("suite ran");

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
            // The syntax suites only emit Pass/Fail/OutOfScope, but the shared enum has
            // other variants; render any as OTHER for the set diff (they cannot occur here).
            _ => "OTHER",
        };
        lines.push(format!("{tag}\t{}", r.name));
        if tag == "FAIL" {
            if let Outcome::Fail(e) = &r.outcome {
                eprintln!("FAIL {}: {e}", r.name);
            }
        }
    }
    lines.sort();

    println!("=== {} outcome set ===", suite.label());
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

    // The RATCHET: pass count may only rise (a fall is a parser regression — or, for a
    // future generated candidate wired into these paths, a disqualifying divergence).
    assert!(
        out.len() >= floor,
        "{} suite shrank: ran {} < floor {floor}",
        suite.label(),
        out.len()
    );
    assert!(
        pass >= floor,
        "{} ratchet: pass {pass} fell below the recorded floor {floor} ({fail} FAIL, {oos} OOS)",
        suite.label()
    );
}

#[test]
fn w3c_ntriples_syntax_ratchet() {
    run_ratchet(LineSuite::NTriples, "rdf-n-triples", NT_SYNTAX_FLOOR);
}

#[test]
fn w3c_nquads_syntax_ratchet() {
    run_ratchet(LineSuite::NQuads, "rdf-n-quads", NQ_SYNTAX_FLOOR);
}

#[test]
fn w3c_trig_syntax_ratchet() {
    run_ratchet(LineSuite::TriG, "rdf-trig", TRIG_SYNTAX_FLOOR);
}
