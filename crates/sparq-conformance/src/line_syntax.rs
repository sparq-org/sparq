//! [FABLE-5] (sq-tonhr.2) The W3C **rdf-n-triples**, **rdf-n-quads** and **rdf-trig**
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

use crate::inference::report::{Outcome, TestResult};
use crate::quadset::{compare_quad_sets, dataset_quads, quad_strings, SetCompare};
use crate::rdf::{iri_to_path, MiniGraph};
use oxrdf::{NamedOrBlankNode, Term};
use std::path::Path;

const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const RDFT: &str = "http://www.w3.org/ns/rdftest#";

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

fn canonical_iri(suite: LineSuite, suite_root: &Path, file: &Path) -> String {
    match file.strip_prefix(suite_root) {
        Ok(rel) => format!("{}{}", suite.canonical_base(), rel.display()),
        Err(_) => crate::rdf::file_iri(file),
    }
}

/// Walk the suite's `manifest.ttl` and run every entry through the sparq parse path.
/// `suite_root` is `<rdf-tests>/rdf/rdf11/rdf-{n-triples,n-quads,trig}`.
pub fn run_suite(
    suite: LineSuite,
    suite_root: &Path,
    out: &mut Vec<TestResult>,
) -> Result<(), String> {
    let manifest = suite_root.join("manifest.ttl");
    if !manifest.is_file() {
        return Err(format!(
            "{} not found — run scripts/fetch-conformance.sh",
            manifest.display()
        ));
    }
    let g = MiniGraph::load(&manifest)?;
    let mut seen = std::collections::HashSet::new();
    for t in &g.triples {
        let node = t.subject.clone();
        if !seen.insert(node.clone()) {
            continue;
        }
        let kind = g
            .types_of(&node)
            .into_iter()
            .find_map(|ty| ty.strip_prefix(RDFT).map(str::to_string));
        let Some(kind) = kind else { continue };
        let name = g
            .str_object(&node, &format!("{MF}name"))
            .unwrap_or_else(|| match &node {
                NamedOrBlankNode::NamedNode(n) => n
                    .as_str()
                    .rsplit(['#', '/'])
                    .next()
                    .unwrap_or(n.as_str())
                    .to_string(),
                NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
            });
        let file = |pred: &str| {
            g.object(&node, &format!("{MF}{pred}"))
                .and_then(|t| match t {
                    Term::NamedNode(n) => iri_to_path(n.as_str()),
                    _ => None,
                })
        };
        let Some(action) = file("action") else {
            continue;
        };
        let result = file("result");
        let stem = suite.kind_stem();
        let outcome = match kind.strip_prefix(stem) {
            Some("PositiveSyntax") => syntax_test(suite, &action, suite_root, true),
            Some("NegativeSyntax") => syntax_test(suite, &action, suite_root, false),
            Some("Eval") => eval_test(suite, &action, result.as_deref(), suite_root, true),
            Some("NegativeEval") => eval_test(suite, &action, result.as_deref(), suite_root, false),
            _ => Outcome::OutOfScope(format!("unhandled rdft type {kind}")),
        };
        out.push(TestResult {
            suite: suite.label().to_string(),
            name,
            outcome,
        });
    }
    Ok(())
}

fn parse_action(
    suite: LineSuite,
    action: &Path,
    suite_root: &Path,
) -> Result<Vec<[String; 4]>, Outcome> {
    let bytes = std::fs::read(action)
        .map_err(|e| Outcome::Fail(format!("read {}: {e}", action.display())))?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let base = canonical_iri(suite, suite_root, action);
    parse_sparq_quads(suite, &text, &base).map_err(Outcome::Fail)
}

fn syntax_test(suite: LineSuite, action: &Path, suite_root: &Path, positive: bool) -> Outcome {
    match parse_action(suite, action, suite_root) {
        Ok(_) if positive => Outcome::Pass,
        Ok(_) => Outcome::Fail("negative syntax: parser accepted an invalid document".into()),
        Err(Outcome::Fail(_)) if !positive => Outcome::Pass,
        Err(o) if !positive => o, // a read error, not a rejection — surface it
        Err(Outcome::Fail(e)) => Outcome::Fail(format!("parse error: {e}")),
        Err(o) => o,
    }
}

fn eval_test(
    suite: LineSuite,
    action: &Path,
    result: Option<&Path>,
    suite_root: &Path,
    positive: bool,
) -> Outcome {
    let parsed = match parse_action(suite, action, suite_root) {
        Ok(q) => q,
        Err(Outcome::Fail(e)) => {
            return if positive {
                Outcome::Fail(format!("action parse error: {e}"))
            } else {
                Outcome::Pass // rejecting the bad document is the right answer
            };
        }
        Err(o) => return o,
    };
    if !positive {
        return Outcome::Fail("negative eval: parser accepted an invalid document".into());
    }
    let Some(result) = result else {
        return Outcome::Fail(format!("{}Eval without mf:result", suite.kind_stem()));
    };
    // Expected side: the N-Quads expectation via oxttl (the differential reference for
    // the dataset value). Its IRIs are already absolute (they bake in the canonical base).
    let bytes = match std::fs::read(result) {
        Ok(b) => b,
        Err(e) => return Outcome::Fail(format!("read {}: {e}", result.display())),
    };
    let mut expected: Vec<[String; 4]> = Vec::new();
    for q in oxttl::NQuadsParser::new().for_slice(&bytes) {
        match q {
            Ok(q) => expected.push(quad_strings(&q)),
            Err(e) => return Outcome::Fail(format!("expected parse error: {e}")),
        }
    }
    match compare_quad_sets(&parsed, &expected) {
        SetCompare::Equal => Outcome::Pass,
        SetCompare::Different { only_a, only_b } => Outcome::Fail(format!(
            "parsed dataset not isomorphic to the reference ({} vs {} quads; only-parsed {:?}; \
             only-expected {:?})",
            parsed.len(),
            expected.len(),
            only_a,
            only_b,
        )),
        SetCompare::Unverified => {
            Outcome::OutOfScope("blank-node isomorphism budget exhausted (unverified)".into())
        }
    }
}
