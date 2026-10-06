//! [OPUS-4.8] (B4) The W3C **rdf-turtle** test suite (`rdf/rdf11/rdf-turtle/manifest.ttl` in
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
//! relative IRIs against the canonical base `http://www.w3.org/2013/TurtleTests/<file>` baked into
//! the expectation files, with an offline mapping back to the pinned clone.

use crate::inference::report::{Outcome, TestResult};
use crate::rdf::{iri_to_path, MiniGraph};
use oxrdf::{NamedOrBlankNode, Term};
use rustc_hash::FxHashMap;
use std::path::Path;

const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const RDFT: &str = "http://www.w3.org/ns/rdftest#";
/// The canonical base the W3C Turtle suite resolves against — the manifest's own
/// `mf:assumedTestBase <https://w3c.github.io/rdf-tests/rdf/rdf11/rdf-turtle/>`, which the
/// expectation `.nt` files bake into their resolved IRIs. The expected side (an `.nt` of fully
/// resolved absolute IRIs) is parsed by oxttl with no base, so only the ACTION needs this base.
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

fn canonical_iri(tests_root: &Path, file: &Path) -> String {
    match file.strip_prefix(tests_root) {
        Ok(rel) => format!("{CANONICAL_BASE}{}", rel.display()),
        Err(_) => crate::rdf::file_iri(file),
    }
}

/// Walk `rdf/rdf11/rdf-turtle/manifest.ttl` and run every entry through the sparq Turtle path.
/// `turtle_root` is `<rdf-tests>/rdf/rdf11/rdf-turtle`.
pub fn run_suite(turtle_root: &Path, out: &mut Vec<TestResult>) -> Result<(), String> {
    let manifest = turtle_root.join("manifest.ttl");
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
        let outcome = match kind.as_str() {
            "TestTurtlePositiveSyntax" => syntax_test(&action, turtle_root, true),
            "TestTurtleNegativeSyntax" => syntax_test(&action, turtle_root, false),
            "TestTurtleEval" => eval_test(&action, result.as_deref(), turtle_root, true),
            "TestTurtleNegativeEval" => eval_test(&action, result.as_deref(), turtle_root, false),
            other => Outcome::OutOfScope(format!("unhandled rdft type {other}")),
        };
        out.push(TestResult {
            suite: "rdf-turtle".to_string(),
            name,
            outcome,
        });
    }
    Ok(())
}

/// Parse `action` through the sparq Turtle path. Returns `Ok(triples)` or `Err(parse error)`.
fn parse_sparq(action: &Path, turtle_root: &Path) -> Result<Vec<[String; 3]>, Outcome> {
    let bytes = std::fs::read(action)
        .map_err(|e| Outcome::Fail(format!("read {}: {e}", action.display())))?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let base = canonical_iri(turtle_root, action);
    match sparq_core::Graph::parse_to_triples_with_base(&text, "turtle", &base) {
        Ok((dict, ids)) => Ok(ids
            .iter()
            .map(|&[s, p, o]| {
                [
                    dict.term(s).to_string(),
                    dict.term(p).to_string(),
                    dict.term(o).to_string(),
                ]
            })
            .collect()),
        Err(e) => Err(Outcome::Fail(e)),
    }
}

fn syntax_test(action: &Path, turtle_root: &Path, positive: bool) -> Outcome {
    match parse_sparq(action, turtle_root) {
        Ok(_) if positive => Outcome::Pass,
        Ok(_) => Outcome::Fail("negative syntax: parser accepted an invalid document".into()),
        Err(Outcome::Fail(_)) if !positive => Outcome::Pass,
        Err(o) if !positive => o, // a read error, not a rejection — surface it
        Err(Outcome::Fail(e)) => Outcome::Fail(format!("parse error: {e}")),
        Err(o) => o,
    }
}

fn eval_test(action: &Path, result: Option<&Path>, turtle_root: &Path, positive: bool) -> Outcome {
    let parsed = match parse_sparq(action, turtle_root) {
        Ok(t) => t,
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
        return Outcome::Fail("TestTurtleEval without mf:result".into());
    };
    // Expected side: the N-Triples result via oxttl (the differential reference for the graph
    // value). The .nt expectation bakes in the canonical TurtleTests base, so its IRIs already
    // match what the action resolves to.
    let expected = match crate::rdf::parse_file(result) {
        Ok(t) => t
            .iter()
            .map(|t| {
                [
                    nob_to_string(&t.subject),
                    format!("<{}>", t.predicate.as_str()),
                    term_to_string(&t.object),
                ]
            })
            .collect::<Vec<_>>(),
        Err(e) => return Outcome::Fail(format!("expected parse error: {e}")),
    };
    if iso(&parsed, &expected) {
        Outcome::Pass
    } else {
        Outcome::Fail(format!(
            "parsed graph not isomorphic to the reference ({} vs {} triples)",
            parsed.len(),
            expected.len()
        ))
    }
}

fn nob_to_string(n: &NamedOrBlankNode) -> String {
    match n {
        NamedOrBlankNode::NamedNode(n) => format!("<{}>", n.as_str()),
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

/// Match the `Display` shapes sparq's `Dict::term` produces, so the two sides compare directly:
/// IRIs as `<…>`, blank nodes as `_:…`, literals via oxrdf's N-Triples-style `Display`.
fn term_to_string(t: &Term) -> String {
    match t {
        Term::NamedNode(n) => format!("<{}>", n.as_str()),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Blank-node graph isomorphism over string-rendered triples. Blank-node tokens
// (`_:label`) on either side are matched under one bijection; everything else
// compares verbatim.
// ---------------------------------------------------------------------------

fn is_bnode(s: &str) -> bool {
    s.starts_with("_:")
}

fn iso(a: &[[String; 3]], b: &[[String; 3]]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut fwd: FxHashMap<String, String> = FxHashMap::default();
    let mut rev: FxHashMap<String, String> = FxHashMap::default();
    let mut trail: Vec<String> = Vec::new();
    let mut used = vec![false; b.len()];
    iso_search(0, a, b, &mut used, &mut fwd, &mut rev, &mut trail, &mut 0)
}

#[allow(clippy::too_many_arguments)]
fn iso_search(
    i: usize,
    a: &[[String; 3]],
    b: &[[String; 3]],
    used: &mut [bool],
    fwd: &mut FxHashMap<String, String>,
    rev: &mut FxHashMap<String, String>,
    trail: &mut Vec<String>,
    steps: &mut usize,
) -> bool {
    const BUDGET: usize = 2_000_000;
    if i == a.len() {
        return true;
    }
    for j in 0..b.len() {
        if used[j] {
            continue;
        }
        *steps += 1;
        if *steps > BUDGET {
            return false;
        }
        let mark = trail.len();
        if (0..3).all(|k| pos_match(&a[i][k], &b[j][k], fwd, rev, trail)) {
            used[j] = true;
            if iso_search(i + 1, a, b, used, fwd, rev, trail, steps) {
                return true;
            }
            used[j] = false;
        }
        while trail.len() > mark {
            let x = trail.pop().unwrap();
            if let Some(y) = fwd.remove(&x) {
                rev.remove(&y);
            }
        }
    }
    false
}

fn pos_match(
    x: &str,
    y: &str,
    fwd: &mut FxHashMap<String, String>,
    rev: &mut FxHashMap<String, String>,
    trail: &mut Vec<String>,
) -> bool {
    match (is_bnode(x), is_bnode(y)) {
        (true, true) => match (fwd.get(x), rev.get(y)) {
            (Some(mx), Some(my)) => mx == y && my == x,
            (None, None) => {
                fwd.insert(x.to_string(), y.to_string());
                rev.insert(y.to_string(), x.to_string());
                trail.push(x.to_string());
                true
            }
            _ => false,
        },
        (false, false) => x == y,
        _ => false,
    }
}
