//! (B4) The W3C **rdf-turtle** test suite (`rdf/rdf11/rdf-turtle/manifest.ttl` in
//! w3c/rdf-tests), run THROUGH THE SPARQ TURTLE PARSER (`sparq_core::Graph::parse_to_triples`),
//! not oxttl directly and not the N3 parser.
//!
//! This is the rejection / acceptance oracle the Turtle T1 spike (drop the `oxrdf::Term`
//! materialization tax in `parse_turtle_chunk`) is gated on. The existing
//! `parallel_turtle_bnodes_match_serial` oracle in `sparq-core` proves *chunked ≡ serial-oxttl*
//! — it cannot catch sparq accepting (or rejecting) something the W3C grammar does not, because
//! its reference IS oxttl-per-chunk. This suite closes that gap by asserting, against the W3C
//! manifest's own positive/negative classification:
//!
//! - `rdft:TestTurtlePositiveSyntax` — `parse_to_triples("turtle")` MUST accept.
//! - `rdft:TestTurtleNegativeSyntax` — `parse_to_triples("turtle")` MUST reject (the load-bearing
//!   rejection cases: bad IRIs, bad escapes, unterminated strings, bad prefixes, …).
//! - `rdft:TestTurtleEval` — accept AND the parsed graph must be isomorphic to the expected
//!   N-Triples result (oxttl parses the `.nt` expectation; comparison is a blank-node-renamed
//!   term-set equality, never line-by-line — bnode labels and number/string canonicalisation
//!   differ across tools).
//! - `rdft:TestTurtleNegativeEval` — MUST reject.
//!
//! Source: w3c/rdf-tests (same pinned clone as `scripts/fetch-conformance.sh`). The suite resolves
//! relative IRIs against the canonical base `https://w3c.github.io/rdf-tests/rdf/rdf11/rdf-turtle/<file>` baked into
//! the expectation files, with an offline mapping back to the pinned clone.

use crate::inference::report::TestResult;
use std::path::Path;

const CANONICAL_BASE: &str = "https://w3c.github.io/rdf-tests/rdf/rdf11/rdf-turtle/";

pub fn notes() -> Vec<String> {
    vec![
        "Source: w3c/rdf-tests `rdf/rdf11/rdf-turtle/manifest.ttl` (pinned clone). Runs THROUGH \
         the sparq Turtle parser (`Graph::parse_to_triples(\"turtle\")`) — the rejection/acceptance \
         oracle for the Turtle parse path, distinct from the oxttl-differential chunked-vs-serial \
         test and from the N3-parser TurtleTests. PositiveSyntax must parse, NegativeSyntax must be \
         rejected, Eval must parse to a graph isomorphic to the N-Triples expectation, NegativeEval \
         must be rejected. Comparison is blank-node-isomorphic term-set equality (never line-by-line)."
            .to_string(),
    ]
}

/// Run one Turtle manifest. `base` overrides its `mf:assumedTestBase` when supplied.
/// Returns each test IRI with its named outcome, including failures to run a test.
pub fn run_manifest(
    manifest: &Path,
    base: Option<&str>,
) -> Result<Vec<(String, TestResult)>, String> {
    crate::syntax_suite::run_manifest(manifest, base, "rdf-turtle", "TestTurtle", |text, base| {
        sparq_core::Graph::load_str_with_base(text, "turtle", base)
    })
}

/// Walk the RDF 1.1 Turtle manifest through the sparq parse path.
pub fn run_suite(turtle_root: &Path, out: &mut Vec<TestResult>) -> Result<(), String> {
    out.extend(
        run_manifest(&turtle_root.join("manifest.ttl"), Some(CANONICAL_BASE))?
            .into_iter()
            .map(|(_, result)| result),
    );
    Ok(())
}
