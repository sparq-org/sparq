//! The w3c/N3 community-group test suite (the manifests EYE and cwm run),
//! mapped onto the `sparq_reason::n3` engine:
//!
//! - `manifest-reasoner.ttl` — `test:TestN3Reason`: run the action document to
//!   its forward closure (`reason_n3_terms`), shape the output per the cwm
//!   options (`think`/`rules` + `data`/`conclusions`), and compare with the
//!   reference graph under blank-node isomorphism (formula terms compared
//!   structurally).
//! - `manifest-parser.ttl` + `manifest-extended.ttl` — N3 syntax (positive =
//!   must parse, negative = must be rejected) and `TestN3Eval` (parse →
//!   isomorphic statements).
//! - `TurtleTests/manifest.ttl` — Turtle through the N3 parser (N3 is a
//!   superset of Turtle): syntax both ways plus eval against the N-Triples
//!   expectation parsed by oxttl.
//!
//! cwm-option semantics honored: `data` drops formula-valued facts and rules
//! from the output; `conclusions` outputs ONLY derived facts; `think` vs
//! `rules` both run to fixpoint here (cwm's `--rules` is single-pass — the
//! suites' reference outputs coincide on these cases or fail loudly);
//! `strings` (log:outputString) is out of scope.

use super::report::{Outcome, TestResult};
use crate::rdf::{as_node, iri_to_path, MiniGraph};
use rustc_hash::FxHashMap;
use sparq_reason::n3::parser::{self, Parsed};
use sparq_reason::n3::Term as NTerm;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const TEST: &str = "https://w3c.github.io/N3/tests/test.n3#";
const RDFT: &str = "http://www.w3.org/ns/rdftest#";
const LOG_IMPLIES: &str = "http://www.w3.org/2000/10/swap/log#implies";
const LOG_IMPLIED_BY: &str = "http://www.w3.org/2000/10/swap/log#isImpliedBy";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

pub fn notes() -> Vec<String> {
    vec![
        "Source: w3c/N3 `tests/` (pinned clone). The reasoner manifest measures EYE/cwm \
         parity of the N3 rule engine; the parser/extended manifests measure the N3 parser \
         (positive = must parse, negative = must be rejected); TurtleTests runs the parser \
         in STRICT Turtle mode. Documents parse against the suite's canonical \
         https://w3c.github.io/N3/tests/ base (the .nt expectations bake those IRIs in), \
         and an offline resolver maps those IRIs back into the pinned clone so \
         log:semantics/log:content work without I/O. Reference graphs are compared under \
         blank-node isomorphism (formulae structurally, lists expanded where the \
         expectation is plain triples, same-datatype numerics by value); reason references \
         parse against the ACTION document's base (cwm generated them so). Under \
         test:conclusions both derived-only and store-minus-rules shapings are accepted \
         (the vendored cwm out-files are inconsistent between the two). `rdft:Rejected` \
         entries are out of scope (upstream rejected them); `test:strings` \
         (log:outputString) stays out of scope."
            .to_string(),
    ]
}

/// The suite's CANONICAL base: w3c/N3 documents and references resolve
/// against `https://w3c.github.io/N3/tests/…` (the TurtleTests `.nt`
/// expectations and the cwm reference outputs both bake those IRIs in).
const CANONICAL_BASE: &str = "https://w3c.github.io/N3/tests/";

fn canonical_iri(tests_root: &Path, file: &Path) -> String {
    match file.strip_prefix(tests_root) {
        Ok(rel) => format!("{CANONICAL_BASE}{}", rel.display()),
        Err(_) => crate::rdf::file_iri(file),
    }
}

/// The [`sparq_reason::n3::Resolver`] for `log:semantics`/`log:content`:
/// canonical suite IRIs map back into the local pinned clone — strictly
/// offline, nothing outside the tests directory is readable.
fn suite_resolver(tests_root: std::path::PathBuf) -> impl Fn(&str) -> Option<String> {
    move |iri: &str| {
        let rel = iri.strip_prefix(CANONICAL_BASE)?;
        let rel = rel.split(['#', '?']).next().unwrap_or(rel);
        if rel.contains("..") {
            return None;
        }
        std::fs::read_to_string(tests_root.join(rel)).ok()
    }
}

pub fn run_suite(n3_root: &Path, out: &mut Vec<TestResult>) -> Result<(), String> {
    let tests = n3_root.join("tests");
    if !tests.is_dir() {
        return Err(format!(
            "{} not found — run scripts/fetch-inference-suites.sh",
            tests.display()
        ));
    }
    for (suite, manifest) in [
        ("n3/reasoner", "N3Tests/manifest-reasoner.ttl"),
        ("n3/parser", "N3Tests/manifest-parser.ttl"),
        ("n3/extended", "N3Tests/manifest-extended.ttl"),
        ("n3/turtle", "TurtleTests/manifest.ttl"),
    ] {
        run_manifest(&tests.join(manifest), suite, &tests, out)?;
    }
    Ok(())
}

fn run_manifest(
    path: &Path,
    suite: &str,
    tests_root: &Path,
    out: &mut Vec<TestResult>,
) -> Result<(), String> {
    let g = MiniGraph::load(path)?;
    // Walk by declared type (robust against the one malformed entries-list
    // token upstream), in manifest order via the subjects' first occurrence.
    let mut seen = std::collections::HashSet::new();
    for t in &g.triples {
        let node = t.subject.clone();
        if !seen.insert(node.clone()) {
            continue;
        }
        let types = g.types_of(&node);
        let kind = types.iter().find_map(|ty| {
            ty.strip_prefix(TEST)
                .or_else(|| ty.strip_prefix(RDFT).and_then(|r| r.strip_prefix("Test")))
        });
        let Some(kind) = kind else { continue };
        let name = g
            .str_object(&node, &format!("{MF}name"))
            .unwrap_or_default();
        let name = if name.is_empty() || suite == "n3/reasoner" {
            // Reasoner mf:names repeat upstream; the entry id is the useful key.
            match &node {
                oxrdf::NamedOrBlankNode::NamedNode(n) => n
                    .as_str()
                    .rsplit(['#', '/'])
                    .next()
                    .unwrap_or(n.as_str())
                    .to_string(),
                oxrdf::NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
            }
        } else {
            name
        };
        let file = |pred: &str| {
            g.object(&node, &format!("{MF}{pred}"))
                .and_then(|t| match t {
                    oxrdf::Term::NamedNode(n) => iri_to_path(n.as_str()),
                    _ => None,
                })
        };
        let Some(action) = file("action") else {
            continue; // subjects without mf:action are manifest scaffolding
        };
        let result = file("result");
        let rejected = g
            .object(&node, "http://www.w3.org/ns/rdftest#approval")
            .is_some_and(
                |t| matches!(t, oxrdf::Term::NamedNode(n) if n.as_str().ends_with("Rejected")),
            );
        let mut outcome = if rejected {
            // rdft:Rejected — upstream explicitly rejected the entry from the
            // suite (e.g. biR's bare '+' list element: "not allowed in
            // turtle nor n3").
            Outcome::OutOfScope("rdft:Rejected upstream".into())
        } else {
            match kind {
                "TestN3PositiveSyntax" => syntax_test(&action, tests_root, true, false),
                "TurtlePositiveSyntax" => syntax_test(&action, tests_root, true, true),
                "TestN3NegativeSyntax" => syntax_test(&action, tests_root, false, false),
                "TurtleNegativeSyntax" => syntax_test(&action, tests_root, false, true),
                "TestN3Eval" => eval_test_n3(&action, result.as_deref(), tests_root),
                "TurtleEval" => eval_test_turtle(&action, result.as_deref(), tests_root, true),
                "TurtleNegativeEval" => {
                    eval_test_turtle(&action, result.as_deref(), tests_root, false)
                }
                "TestN3Reason" => reason_test(&g, &node, &action, result.as_deref(), tests_root),
                other => Outcome::OutOfScope(format!("unhandled test type {other}")),
            }
        };
        if let Outcome::Fail(e) = &outcome {
            if let Some((_, _why, detail)) =
                DOCUMENTED_DIVERGENCES.iter().find(|(n, _, _)| *n == name)
            {
                outcome = Outcome::Divergence(detail, e.clone());
            }
        }
        out.push(TestResult {
            suite: suite.to_string(),
            name,
            outcome,
        });
    }
    Ok(())
}

/// Parse on a watchdog thread (the recursive-descent parser may loop on
/// pathological input — that becomes a recorded FAIL, not a hung harness).
fn parse_with_watchdog(src: String, base: String) -> Result<Result<Parsed, String>, String> {
    parse_with_watchdog_mode(src, base, false)
}

/// `strict` = the W3C Turtle grammar (for the TurtleTests suite).
fn parse_with_watchdog_mode(
    src: String,
    base: String,
    strict: bool,
) -> Result<Result<Parsed, String>, String> {
    let (tx, rx) = mpsc::channel();
    // Generous stack: the stress-test files nest deeply and the parser
    // recurses per nesting level (bounded by its MAX_DEPTH guard).
    let _ = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let result = std::panic::catch_unwind(|| {
                if strict {
                    parser::parse_turtle_with_base(&src, &base)
                } else {
                    parser::parse_with_base(&src, &base)
                }
            });
            let _ = tx.send(result);
        });
    match rx.recv_timeout(TEST_TIMEOUT) {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(_)) => Err("parser panicked".into()),
        Err(mpsc::RecvTimeoutError::Timeout) => Err("timeout (10s) in parser".into()),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("parser panicked".into()),
    }
}

fn read(path: &Path) -> Result<String, Outcome> {
    match std::fs::read(path) {
        // Lossy: one legacy suite file (07test/utf8.n3) carries a stray
        // non-UTF-8 byte; the replacement character keeps it parseable.
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) => Err(Outcome::Fail(format!("read {}: {e}", path.display()))),
    }
}

fn syntax_test(action: &Path, tests_root: &Path, positive: bool, strict: bool) -> Outcome {
    let src = match read(action) {
        Ok(s) => s,
        Err(o) => return o,
    };
    let base = canonical_iri(tests_root, action);
    match parse_with_watchdog_mode(src, base, strict) {
        Ok(Ok(_)) if positive => Outcome::Pass,
        Ok(Ok(_)) => Outcome::Fail("negative syntax: parser accepted an invalid document".into()),
        Ok(Err(_)) if !positive => Outcome::Pass,
        Ok(Err(e)) => Outcome::Fail(format!("parse error: {e}")),
        Err(e) if !positive => {
            // A panic on invalid input is still a rejection, but parsers
            // should reject cleanly — keep it loud.
            Outcome::Fail(format!("rejected via {e} (should error cleanly)"))
        }
        Err(e) => Outcome::Fail(e),
    }
}

/// All statements of a parsed document, with rules re-encoded back into their
/// surface triple form so eval comparisons see them.
fn statements(p: &Parsed) -> Vec<[NTerm; 3]> {
    let quote = |ts: &[[NTerm; 3]]| -> NTerm {
        if ts.is_empty() {
            NTerm::Lit("true".into(), XSD_BOOLEAN.into(), None) // `{}` = true
        } else {
            NTerm::Formula(ts.to_vec())
        }
    };
    let mut stmts = p.facts.clone();
    for r in &p.rules {
        stmts.push([
            quote(&r.premise),
            NTerm::Iri(LOG_IMPLIES.into()),
            quote(&r.conclusion),
        ]);
    }
    for r in &p.backward_rules {
        stmts.push([
            quote(&r.conclusion),
            NTerm::Iri(LOG_IMPLIED_BY.into()),
            quote(&r.premise),
        ]);
    }
    stmts
}

fn eval_test_n3(action: &Path, result: Option<&Path>, tests_root: &Path) -> Outcome {
    let Some(result) = result else {
        return Outcome::Fail("TestN3Eval without mf:result".into());
    };
    let (src, expected_src) = match (read(action), read(result)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(o), _) | (_, Err(o)) => return o,
    };
    let parsed = match parse_with_watchdog(src, canonical_iri(tests_root, action)) {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return Outcome::Fail(format!("action parse error: {e}")),
        Err(e) => return Outcome::Fail(e),
    };
    // cwm generated the reference files against the ACTION document's base
    // (their headers say so) — relative IRIs in a `-ref.n3` resolve against
    // the action, not the ref file itself.
    let expected = match parse_with_watchdog(expected_src, canonical_iri(tests_root, action)) {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return Outcome::Fail(format!("expected parse error: {e}")),
        Err(e) => return Outcome::Fail(e),
    };
    // Lists expand to rdf:first/rest chains on BOTH sides: some expected
    // outputs are N-Triples files spelling the chains out (cwm path2).
    if n3_iso(
        &expand_lists(&statements(&parsed)),
        &expand_lists(&statements(&expected)),
    ) {
        Outcome::Pass
    } else {
        Outcome::Fail("parsed statements not isomorphic to the reference".into())
    }
}

fn eval_test_turtle(
    action: &Path,
    result: Option<&Path>,
    tests_root: &Path,
    positive: bool,
) -> Outcome {
    let src = match read(action) {
        Ok(s) => s,
        Err(o) => return o,
    };
    let parsed = match parse_with_watchdog_mode(src, canonical_iri(tests_root, action), true) {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            return if positive {
                Outcome::Fail(format!("action parse error: {e}"))
            } else {
                Outcome::Pass // rejecting the document certainly avoids the wrong graph
            };
        }
        Err(e) => return Outcome::Fail(e),
    };
    let Some(result) = result else {
        // A NegativeEval without mf:result: the document must not evaluate —
        // accepting it (we parsed fine and there is nothing to compare
        // against) is a failure only for positive tests.
        return if positive {
            Outcome::Fail("eval test without mf:result".into())
        } else {
            Outcome::Fail("negative eval accepted (parsed; no result to refute)".into())
        };
    };
    // Expected side: N-Triples/Turtle ground graph via oxttl.
    let expected = match crate::rdf::parse_file(result) {
        Ok(t) => t,
        Err(e) => return Outcome::Fail(format!("expected parse error: {e}")),
    };
    let expected_stmts: Vec<[NTerm; 3]> = expected
        .iter()
        .map(|t| {
            [
                oxrdf_to_n3(&oxrdf::Term::from(t.subject.clone())),
                oxrdf_to_n3(&oxrdf::Term::NamedNode(t.predicate.clone())),
                oxrdf_to_n3(&t.object),
            ]
        })
        .collect();
    // The N3 parser yields first-class list terms; the N-Triples expectation
    // has rdf:first/rest chains — expand the actual side to plain triples.
    let iso = n3_iso(&expand_lists(&statements(&parsed)), &expected_stmts);
    match (iso, positive) {
        (true, true) | (false, false) => Outcome::Pass,
        (false, true) => Outcome::Fail("parsed graph not isomorphic to the reference".into()),
        (true, false) => Outcome::Fail("negative eval: graphs are isomorphic".into()),
    }
}

/// Expand first-class `Term::List` values into rdf:first/rest blank-node
/// chains (fresh chain per occurrence, like Turtle's `( … )` sugar), so an
/// N3-parsed graph can be compared against a plain-triple expectation.
fn expand_lists(rows: &[[NTerm; 3]]) -> Vec<[NTerm; 3]> {
    const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
    const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
    const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
    fn expand(t: &NTerm, counter: &mut usize, out: &mut Vec<[NTerm; 3]>) -> NTerm {
        match t {
            NTerm::List(ms) if ms.is_empty() => NTerm::Iri(RDF_NIL.into()),
            NTerm::List(ms) => {
                let members: Vec<NTerm> = ms.iter().map(|m| expand(m, counter, out)).collect();
                let mut tail = NTerm::Iri(RDF_NIL.into());
                for m in members.into_iter().rev() {
                    *counter += 1;
                    let node = NTerm::Blank(format!("__xl{counter}"));
                    out.push([node.clone(), NTerm::Iri(RDF_FIRST.into()), m]);
                    out.push([node.clone(), NTerm::Iri(RDF_REST.into()), tail]);
                    tail = node;
                }
                tail
            }
            NTerm::Formula(ts) => {
                let mut inner: Vec<[NTerm; 3]> = Vec::new();
                let mut rows: Vec<[NTerm; 3]> = Vec::new();
                for r in ts {
                    let nr = [
                        expand(&r[0], counter, &mut inner),
                        expand(&r[1], counter, &mut inner),
                        expand(&r[2], counter, &mut inner),
                    ];
                    rows.push(nr);
                }
                rows.append(&mut inner);
                NTerm::Formula(rows)
            }
            // A quoted `<< s p o >>` triple term is transparent structure:
            // recurse so a list/formula nested in a component still expands
            // (GH #2012). [FABLE-5]
            NTerm::Triple(tr) => NTerm::Triple(Box::new([
                expand(&tr[0], counter, out),
                expand(&tr[1], counter, out),
                expand(&tr[2], counter, out),
            ])),
            _ => t.clone(),
        }
    }
    let mut counter = 0usize;
    let mut out: Vec<[NTerm; 3]> = Vec::new();
    let mut structure: Vec<[NTerm; 3]> = Vec::new();
    for r in rows {
        let nr = [
            expand(&r[0], &mut counter, &mut structure),
            expand(&r[1], &mut counter, &mut structure),
            expand(&r[2], &mut counter, &mut structure),
        ];
        out.push(nr);
    }
    out.append(&mut structure);
    out
}

fn oxrdf_to_n3(t: &oxrdf::Term) -> NTerm {
    match t {
        oxrdf::Term::NamedNode(n) => NTerm::Iri(n.as_str().to_string()),
        oxrdf::Term::BlankNode(b) => NTerm::Blank(b.as_str().to_string()),
        oxrdf::Term::Literal(l) => NTerm::Lit(
            l.value().to_string(),
            l.datatype().as_str().to_string(),
            l.language().map(|s| s.to_string()),
        ),
        // RDF 1.2 quoted-triple term: convert faithfully now the N3 model has
        // a first-class `NTerm::Triple` (GH #2012). [FABLE-5]
        oxrdf::Term::Triple(t) => NTerm::Triple(Box::new([
            match &t.subject {
                oxrdf::NamedOrBlankNode::NamedNode(n) => NTerm::Iri(n.as_str().to_string()),
                oxrdf::NamedOrBlankNode::BlankNode(b) => NTerm::Blank(b.as_str().to_string()),
            },
            NTerm::Iri(t.predicate.as_str().to_string()),
            oxrdf_to_n3(&t.object),
        ])),
    }
}

// ---------------------------------------------------------------------------
// TestN3Reason
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Options {
    data: bool,
    conclusions: bool,
    strings: bool,
    filter: bool,
}

fn options(g: &MiniGraph, node: &oxrdf::NamedOrBlankNode) -> Options {
    let mut o = Options::default();
    if let Some(opt_t) = g.object(node, &format!("{TEST}options")) {
        if let Some(opt) = as_node(opt_t) {
            let flag = |name: &str| {
                matches!(
                    g.str_object(&opt, &format!("{TEST}{name}")).as_deref(),
                    Some("true") | Some("1")
                )
            };
            o.data = flag("data");
            o.conclusions = flag("conclusions");
            o.strings = flag("strings");
            o.filter = g.object(&opt, &format!("{TEST}filter")).is_some();
        }
    }
    o
}

/// Suite entries whose REFERENCE disagrees with their action document —
/// failing them is not an engine gap. Kept as documented divergences.
const DOCUMENTED_DIVERGENCES: &[(&str, &str, &str)] = &[
    (
        "bad_prefix2",
        "suite-internal conflict",
        "extra/bad_prefix2.n3 expects an UNDECLARED ':' prefix to be rejected, but the \
         reasoner manifest's own cwm actions (e.g. cwm_unify/unify1.n3) rely on cwm's \
         undeclared-':'-means-<#> convention — the engine keeps the cwm behavior",
    ),
    (
        "numbers.n3",
        "upstream ref base mixup",
        "the expected output cwm_n3/n3parser.tests_n3_10013.n3 keeps ONE statement under \
         the generating author's local base (file:/home/syosi/...#is) while the rest were \
         rebased — that statement can never match any correct parse",
    ),
    (
        "cwm_includes_t11",
        "vendored ref reflects a failed dereference",
        "with the offline resolver the engine derives the schema-checking conclusions over \
         <t10a.n3> (test_undefined etc.); t11-ref.n3 holds only the two foo.n3 conclusions \
         and even omits t11's own data facts — the cwm run that produced it never resolved \
         t10a.n3 and ran with an unrecorded --purge",
    ),
    (
        "cwm_unify_unify1",
        "upstream ref/action mismatch",
        "the action concludes `:test :a ?x` (predicate <unify1.n3#a>) but unify1-ref.n3 \
         says `:test a :Successful` (rdf:type) — the vendored cwm reference was generated \
         from an older revision of the action",
    ),
    (
        "cwm_includes_conclusion",
        "upstream ref from older sources, and not deductively closed",
        "the engine now derives the `:result :is { … }` conclusion formula, but the \
         vendored conclusion-ref.n3 cannot match the vendored sources: (1) its quoted \
         daml:comment for :Animal reads `…number of\\nontological…` while the vendored \
         daml-ex.n3 has `…number of\\n\\tontological…` (TAB) — the 2003 cwm run used an \
         older daml-ex.n3 revision; (2) the ref formula is not closed under its OWN quoted \
         rules (it holds `d:father daml:range d:Man`, `daml:range = rdfs:range` and the \
         rule `{?x ?p1 ?y. ?p1 = ?p2} => {?x ?p2 ?y}`, yet lacks `d:father rdfs:range \
         d:Man`) — log:conclusion (\"all statements which can be deduced\") legitimately \
         derives 31 statements the 2003 run did not (instantiated transitivity rules and \
         the consequent type/schema facts)",
    ),
];

fn reason_test(
    g: &MiniGraph,
    node: &oxrdf::NamedOrBlankNode,
    action: &Path,
    result: Option<&Path>,
    tests_root: &Path,
) -> Outcome {
    let opts = options(g, node);
    if opts.strings {
        return Outcome::OutOfScope("log:outputString (test:strings) not implemented".into());
    }
    if opts.filter {
        return Outcome::OutOfScope("test:filter (apply-and-replace) not implemented".into());
    }
    let Some(result) = result else {
        return Outcome::Fail("TestN3Reason without mf:result".into());
    };
    let (src, expected_src) = match (read(action), read(result)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(o), _) | (_, Err(o)) => return o,
    };

    // Closure on a watchdog thread (with the offline suite resolver enabling
    // log:semantics/log:content over the pinned clone).
    let base = canonical_iri(tests_root, action);
    let (tx, rx) = mpsc::channel();
    {
        let (src, base) = (src.clone(), base.clone());
        let root = tests_root.to_path_buf();
        let _ = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                let resolver = suite_resolver(root);
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    sparq_reason::n3::reason_n3_terms_with_resolver(
                        &src,
                        Some(&base),
                        Some(&resolver),
                    )
                }));
                let _ = tx.send(r);
            });
    }
    let closure = match rx.recv_timeout(TEST_TIMEOUT) {
        Ok(Ok(Ok(c))) => c,
        Ok(Ok(Err(e))) => return Outcome::Fail(format!("reasoner error: {e}")),
        Ok(Err(_)) => return Outcome::Fail("reasoner panicked".into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Outcome::Fail("timeout (10s) in reasoner".into())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Outcome::Fail("reasoner panicked".into())
        }
    };

    // Shape the output per the cwm options. The vendored cwm `-out.n3`/-ref
    // references are INCONSISTENT for identical `test:conclusions` options
    // (string/contains-out.n3 holds only the derivations; string/roughly-out.n3
    // holds data + derivations — the out-files were produced by different cwm
    // command lines than the manifest reconstruction suggests), so under
    // `conclusions` BOTH shapings are acceptable; `data` drops formula-valued
    // statements; the default output re-adds the rule statements.
    let data_filter = |rows: &mut Vec<[NTerm; 3]>| {
        if opts.data {
            rows.retain(|row| !row.iter().any(|t| matches!(t, NTerm::Formula(_))));
        }
    };
    let mut variants: Vec<Vec<[NTerm; 3]>> = Vec::new();
    if opts.conclusions {
        let mut derived = closure.derived.clone();
        data_filter(&mut derived);
        dedup(&mut derived);
        variants.push(derived);
        let mut full = closure.facts.clone();
        data_filter(&mut full);
        dedup(&mut full);
        variants.push(full);
    } else {
        let mut full = closure.facts.clone();
        data_filter(&mut full);
        dedup(&mut full);
        if !opts.data {
            // Full-store output keeps the document's rules as statements.
            let parsed = match parse_with_watchdog(src, base.clone()) {
                Ok(Ok(p)) => p,
                Ok(Err(e)) => return Outcome::Fail(format!("action parse error: {e}")),
                Err(e) => return Outcome::Fail(e),
            };
            full.extend(statements(&Parsed {
                facts: Vec::new(),
                rules: parsed.rules,
                backward_rules: parsed.backward_rules,
                base: String::new(),
            }));
        }
        variants.push(full);
    }

    // cwm generated the reference against the ACTION's base (see headers).
    let expected = match parse_with_watchdog(expected_src, base) {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return Outcome::Fail(format!("expected parse error: {e}")),
        Err(e) => return Outcome::Fail(e),
    };
    let mut expected_stmts = statements(&expected);
    dedup(&mut expected_stmts);

    if variants.iter().any(|v| n3_iso(v, &expected_stmts)) {
        Outcome::Pass
    } else {
        let actual = &variants[0];
        Outcome::Fail(format!(
            "output not isomorphic to the reference ({} vs {} statements{})",
            actual.len(),
            expected_stmts.len(),
            diff_hint(actual, &expected_stmts)
        ))
    }
}

fn dedup(rows: &mut Vec<[NTerm; 3]>) {
    let mut seen = std::collections::HashSet::new();
    rows.retain(|r| seen.insert(format!("{r:?}")));
}

/// A small label-sensitive sample of one-side-only statements (debug hint).
fn diff_hint(actual: &[[NTerm; 3]], expected: &[[NTerm; 3]]) -> String {
    let fmt = |r: &[NTerm; 3]| format!("{r:?}");
    let a: Vec<String> = actual.iter().map(fmt).collect();
    let e: Vec<String> = expected.iter().map(fmt).collect();
    let mut s = String::new();
    if let Some(m) = e.iter().find(|x| !a.contains(x)) {
        s.push_str(&format!("; expected-only e.g. {m}"));
    }
    if let Some(m) = a.iter().find(|x| !e.contains(x)) {
        s.push_str(&format!("; actual-only e.g. {m}"));
    }
    s.truncate(400);
    s
}

// ---------------------------------------------------------------------------
// N3-term graph isomorphism (blank nodes AND variables under one bijection;
// formula terms compared as recursive statement multisets).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Bij {
    fwd: FxHashMap<String, String>,
    rev: FxHashMap<String, String>,
    trail: Vec<String>,
}

impl Bij {
    fn bind(&mut self, a: String, b: String) -> bool {
        match (self.fwd.get(&a), self.rev.get(&b)) {
            (Some(x), Some(y)) => x == &b && y == &a,
            (None, None) => {
                self.fwd.insert(a.clone(), b.clone());
                self.rev.insert(b, a.clone());
                self.trail.push(a);
                true
            }
            _ => false,
        }
    }
    fn mark(&self) -> usize {
        self.trail.len()
    }
    fn undo(&mut self, mark: usize) {
        while self.trail.len() > mark {
            let a = self.trail.pop().unwrap();
            if let Some(b) = self.fwd.remove(&a) {
                self.rev.remove(&b);
            }
        }
    }
}

const ISO_BUDGET: usize = 2_000_000;

pub(crate) fn n3_iso(a: &[[NTerm; 3]], b: &[[NTerm; 3]]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut bij = Bij::default();
    let mut used = vec![false; b.len()];
    let mut steps = 0usize;
    iso_search(0, a, b, &mut used, &mut bij, &mut steps)
}

fn iso_search(
    i: usize,
    a: &[[NTerm; 3]],
    b: &[[NTerm; 3]],
    used: &mut [bool],
    bij: &mut Bij,
    steps: &mut usize,
) -> bool {
    if i == a.len() {
        return true;
    }
    for j in 0..b.len() {
        if used[j] {
            continue;
        }
        *steps += 1;
        if *steps > ISO_BUDGET {
            return false;
        }
        let mark = bij.mark();
        if (0..3).all(|k| term_iso(&a[i][k], &b[j][k], bij, steps)) {
            used[j] = true;
            if iso_search(i + 1, a, b, used, bij, steps) {
                return true;
            }
            used[j] = false;
        }
        bij.undo(mark);
    }
    false
}

fn term_iso(a: &NTerm, b: &NTerm, bij: &mut Bij, steps: &mut usize) -> bool {
    match (a, b) {
        (NTerm::Blank(x), NTerm::Blank(y)) => bij.bind(format!("b:{x}"), format!("b:{y}")),
        (NTerm::Var(x), NTerm::Var(y)) => bij.bind(format!("v:{x}"), format!("v:{y}")),
        (NTerm::Formula(x), NTerm::Formula(y)) => {
            if x.len() != y.len() {
                return false;
            }
            let mut used = vec![false; y.len()];
            iso_search(0, x, y, &mut used, bij, steps)
        }
        // First-class lists: ordered, member by member.
        (NTerm::List(x), NTerm::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| term_iso(p, q, bij, steps))
        }
        // Quoted `<< s p o >>` triple terms: transparent structure, component
        // by component (so a blank inside one still joins the bijection —
        // GH #2012). [FABLE-5]
        (NTerm::Triple(x), NTerm::Triple(y)) => x
            .iter()
            .zip(y.iter())
            .all(|(p, q)| term_iso(p, q, bij, steps)),
        // Same-datatype numeric literals compare by VALUE (cwm's serializer
        // and the engine may format the same number differently: 0.0e0 = 0e0,
        // 4.0 = 4.00).
        (NTerm::Lit(x, dtx, None), NTerm::Lit(y, dty, None)) if dtx == dty => {
            x == y
                || (is_numeric_dt(dtx) && num_lex(x).zip(num_lex(y)).is_some_and(|(a, b)| a == b))
        }
        _ => a == b,
    }
}

fn is_numeric_dt(dt: &str) -> bool {
    matches!(
        dt.strip_prefix("http://www.w3.org/2001/XMLSchema#"),
        Some("integer" | "decimal" | "double" | "float")
    )
}

fn num_lex(s: &str) -> Option<f64> {
    match s {
        "INF" | "+INF" => Some(f64::INFINITY),
        "-INF" => Some(f64::NEG_INFINITY),
        _ => s.parse::<f64>().ok().filter(|v| !v.is_nan()),
    }
}
