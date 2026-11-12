//! (sq-tonhr.2) The W3C **rdf-n-triples**, **rdf-n-quads** and **rdf-trig**
//! syntax suites (`rdf/rdf11/rdf-{n-triples,n-quads,trig}/manifest.ttl` in w3c/rdf-tests),
//! run THROUGH THE REAL SPARQ PARSE PATHS — the same entry points `sparq-cli` /
//! `sparq-server` ingest through:
//!
//! - N-Triples: [`sparq_core::Graph::parse_to_triples`]`(_, "ntriples")` — under the
//!   default `parallel` feature this is the NATIVE chunk-parallel `nt.rs` parser (the
//!   incumbent any generated candidate parser must beat, epic sq-tonhr).
//! - N-Quads: [`sparq_core::Graph::load_dataset`]`(_, "nquads")` — the chunk-parallel
//!   oxttl-per-chunk dataset loader, named graphs preserved.
//! - TriG: [`sparq_core::Graph::load_dataset_with_base`]`(_, "trig", base)` — the serial
//!   with-base dataset path (the W3C actions carry relative IRIs against the manifest's
//!   `mf:assumedTestBase`), named graphs preserved.
//!
//! Test kinds per the manifests' own positive/negative classification:
//! `rdft:Test{NTriples,NQuads,Trig}PositiveSyntax` MUST parse,
//! `rdft:Test{NTriples,NQuads,Trig}NegativeSyntax` MUST be rejected,
//! `rdft:TestTrigEval` MUST parse to a dataset QUAD-SET-isomorphic (blank-node bijection,
//! graph names included) to the N-Quads expectation, `rdft:TestTrigNegativeEval` MUST be
//! rejected. Before sq-tonhr.2 only rdf-turtle was ratcheted (`turtle_suite`); these three
//! suites pin the bar the epic's generated NT/NQ/TriG parsers are differential-gated
//! against (no candidate may change any suite's outcome set).
//!
//! Source: w3c/rdf-tests (same pinned clone as `scripts/fetch-conformance.sh`).

use crate::inference::report::TestResult;
use crate::quadset::dataset_quads;
use std::path::Path;

/// Which of the three line-oriented suites to run — selects the manifest's test kinds,
/// the parse path and (for TriG) the canonical base handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineSuite {
    /// `rdf/rdf11/rdf-n-triples` through the native chunk-parallel `nt.rs` path.
    NTriples,
    /// `rdf/rdf11/rdf-n-quads` through the chunk-parallel dataset loader.
    NQuads,
    /// `rdf/rdf11/rdf-trig` through the serial with-base dataset loader.
    TriG,
}

impl LineSuite {
    /// The suite's report label (also the `TestResult::suite` value).
    pub fn label(self) -> &'static str {
        match self {
            LineSuite::NTriples => "rdf-n-triples",
            LineSuite::NQuads => "rdf-n-quads",
            LineSuite::TriG => "rdf-trig",
        }
    }

    /// The canonical base the suite's actions resolve against — the manifest's own
    /// `mf:assumedTestBase`. Only TriG documents can contain relative IRIs; the
    /// line-based formats require absolute IRIs, so their base is never consulted.
    fn canonical_base(self) -> &'static str {
        match self {
            LineSuite::NTriples => "https://w3c.github.io/rdf-tests/rdf/rdf11/rdf-n-triples/",
            LineSuite::NQuads => "https://w3c.github.io/rdf-tests/rdf/rdf11/rdf-n-quads/",
            LineSuite::TriG => "https://w3c.github.io/rdf-tests/rdf/rdf11/rdf-trig/",
        }
    }

    /// The `rdft:` test-kind stem, e.g. `TestTrig` for `TestTrigPositiveSyntax`.
    fn kind_stem(self) -> &'static str {
        match self {
            LineSuite::NTriples => "TestNTriples",
            LineSuite::NQuads => "TestNQuads",
            LineSuite::TriG => "TestTrig",
        }
    }
}

/// Parse `text` through the suite's REAL sparq entry point, returning the resulting
/// quad set (rendered) or the parse error.
pub fn parse_sparq_quads(
    suite: LineSuite,
    text: &str,
    base: &str,
) -> Result<Vec<[String; 4]>, String> {
    match suite {
        LineSuite::NTriples => {
            let (dict, ids) = sparq_core::Graph::parse_to_triples(text, "ntriples")?;
            Ok(ids
                .iter()
                .map(|&[s, p, o]| {
                    [
                        dict.term(s).to_string(),
                        dict.term(p).to_string(),
                        dict.term(o).to_string(),
                        String::new(),
                    ]
                })
                .collect())
        }
        LineSuite::NQuads => {
            sparq_core::Graph::load_dataset(text, "nquads").map(|g| dataset_quads(&g))
        }
        LineSuite::TriG => {
            sparq_core::Graph::load_dataset_with_base(text, "trig", base).map(|g| dataset_quads(&g))
        }
    }
}

/// Run one manifest with its declared base, or an explicit base override.
/// Returns each test IRI with its named outcome, including failures to run a test.
pub fn run_manifest(
    suite: LineSuite,
    manifest: &Path,
    base: Option<&str>,
) -> Result<Vec<(String, TestResult)>, String> {
    match suite {
        LineSuite::NTriples => crate::syntax_suite::run_manifest(
            manifest,
            base,
            suite.label(),
            suite.kind_stem(),
            |text, base| parse_sparq_quads(LineSuite::NTriples, text, base),
        ),
        LineSuite::NQuads => crate::syntax_suite::run_manifest(
            manifest,
            base,
            suite.label(),
            suite.kind_stem(),
            |text, base| parse_sparq_quads(LineSuite::NQuads, text, base),
        ),
        LineSuite::TriG => crate::syntax_suite::run_manifest(
            manifest,
            base,
            suite.label(),
            suite.kind_stem(),
            |text, base| parse_sparq_quads(LineSuite::TriG, text, base),
        ),
    }
}

/// Walk the suite's RDF 1.1 manifest through the sparq parse path.
pub fn run_suite(
    suite: LineSuite,
    suite_root: &Path,
    out: &mut Vec<TestResult>,
) -> Result<(), String> {
    out.extend(
        run_manifest(
            suite,
            &suite_root.join("manifest.ttl"),
            Some(suite.canonical_base()),
        )?
        .into_iter()
        .map(|(_, result)| result),
    );
    Ok(())
}
