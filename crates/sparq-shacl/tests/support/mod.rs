//! Helpers shared by the W3C SHACL suite runners (`mod support;`).
//!
//! The EARL sink: when `SPARQ_EARL_OUT` names a file, each runner appends one
//! `<test IRI>\t<passed|failed>` line per manifest entry it ran.
//! `scripts/earl-report.py` turns those lines into the EARL implementation report
//! the W3C Data Shapes WG collects (`shacl12-test-suite/reports/`). Unset (CI and
//! ordinary local runs), `earl` does nothing.

// Each runner binary uses a subset of these helpers.
#![allow(dead_code)]

use std::io::Write;

/// One manifest entry's outcome in a runner's scoreboard.
pub enum Outcome {
    Pass,
    Fail(String),
    Skip(String),
}

/// Appends `outcome` for `test_iri` to `SPARQ_EARL_OUT` (a no-op when unset).
///
/// A skip because the entry is of another test type (`not sht:…`) belongs to a
/// different runner and is not recorded. Any other skip means sparq could not run
/// the entry, which the report states as a failure.
pub fn earl(test_iri: &str, outcome: &Outcome) {
    let Some(path) = std::env::var_os("SPARQ_EARL_OUT") else {
        return;
    };
    let verdict = match outcome {
        Outcome::Pass => "passed",
        Outcome::Skip(why) if why.starts_with("not sht:") => return,
        Outcome::Fail(_) | Outcome::Skip(_) => "failed",
    };
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap_or_else(|e| panic!("open SPARQ_EARL_OUT {}: {e}", path.to_string_lossy()));
    writeln!(f, "{test_iri}\t{verdict}").expect("write SPARQ_EARL_OUT");
}

/// `mf:result sht:Failure`: a conformant processor must reject the shapes graph
/// (for example an unsound SHACL-SPARQL pre-binding) instead of producing a
/// report. Returns `None` for any other expected result, otherwise whether
/// `validate_strict` rejected.
pub fn expected_failure(
    expected: &oxrdf::Term,
    data: &sparq_core::Graph,
    shapes: &sparq_core::Graph,
) -> Option<bool> {
    let failure = "http://www.w3.org/ns/shacl-test#Failure";
    matches!(expected, oxrdf::Term::NamedNode(n) if n.as_str() == failure)
        .then(|| sparq_shacl::validate_strict(data, shapes).is_err())
}

pub const NOT_REJECTED: &str =
    "expected sht:Failure (validator must reject) but strict validation produced a report";

/// The SHACL 1.0 suite uses its own Appendix A admission policy.
pub fn expected_failure_shacl10(
    expected: &oxrdf::Term,
    data: &sparq_core::Graph,
    shapes: &sparq_core::Graph,
) -> Option<bool> {
    matches!(expected, oxrdf::Term::NamedNode(n) if n.as_str() == "http://www.w3.org/ns/shacl-test#Failure")
        .then(|| sparq_shacl::validate_strict_shacl10(data, shapes).is_err())
}
