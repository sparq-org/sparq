//! Manifest-driven runner for the W3C SHACL core test suite
//! (w3c/data-shapes, `data-shapes-test-suite/tests/core/...`).
//!
//! The suite is fetched by `./fetch-shacl-tests.sh` into `tests/shacl/`
//! (gitignored); when absent this test SKIPS itself so `cargo test --workspace`
//! stays green on a fresh checkout. Manifest-walking helpers are modelled on
//! `sparq-conformance`'s (copied, not shared — that crate is dev-only and must
//! not become a dependency).
//!
//! Comparison policy per `sht:Validate` entry: `sh:conforms` must match; the
//! result multisets must correspond 1:1 where each expected result's stated
//! `sh:focusNode` / `sh:resultPath` (structural) / `sh:value` /
//! `sh:resultSeverity` / `sh:sourceConstraintComponent` / `sh:sourceShape` /
//! `sh:resultMessage` all agree (blank nodes match any blank node — they have
//! no cross-graph identity).

use oxrdf::Term;
use sparq_core::Graph;
use sparq_shacl::view::GraphView;
use sparq_shacl::{validate, Path, ValidationResult};
use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const SHT: &str = "http://www.w3.org/ns/shacl-test#";
const SH: &str = "http://www.w3.org/ns/shacl#";

/// Pass-rate floor: the rate achieved at the pinned suite commit (98/98).
/// Regressions below this fail the build.
/// sq-z1xv8 — the VALUE now lives once in the zero-dependency
/// `sparq-conformance-floors` crate, which `sparq-conformance`'s central
/// `scoreboard::SUITES` reads too, so the enforced floor and the reported floor are
/// ONE `const` and cannot drift (replacing the old textual re-read of this file).
/// Raise it THERE; the measurement narrative stays here.
const BASELINE_PASS: usize = sparq_conformance_floors::shacl::CORE_FLOOR;

fn suite_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/shacl/data-shapes/data-shapes-test-suite/tests/core")
}

#[test]
fn w3c_shacl_core_suite() {
    let root = suite_root();
    if !root.exists() {
        eprintln!(
            "SKIP: W3C SHACL suite not present at {} — run crates/sparq-shacl/fetch-shacl-tests.sh",
            root.display()
        );
        return;
    }
    let mut score = Scoreboard::default();
    walk_manifest(&root.join("manifest.ttl"), &root, &mut score);

    let mut pass = 0;
    let mut fail = 0;
    let mut skip = 0;
    println!("\nW3C SHACL core suite scoreboard");
    println!("{:<22} {:>5} {:>5} {:>5}", "suite", "pass", "fail", "skip");
    for (suite, (p, f, s)) in &score.by_suite {
        println!("{suite:<22} {p:>5} {f:>5} {s:>5}");
        pass += p;
        fail += f;
        skip += s;
    }
    let total = pass + fail;
    println!("{:<22} {pass:>5} {fail:>5} {skip:>5}", "TOTAL");
    if total > 0 {
        println!(
            "pass rate: {pass}/{total} ({:.1}%)",
            100.0 * pass as f64 / total as f64
        );
    }
    if !score.failures.is_empty() {
        println!("\nfailing tests:");
        for (id, why) in &score.failures {
            println!("  {id}: {why}");
        }
    }
    if !score.skips.is_empty() {
        println!("\nskipped tests:");
        for (id, why) in &score.skips {
            println!("  {id}: {why}");
        }
    }
    assert!(
        pass >= BASELINE_PASS,
        "SHACL core pass count regressed: {pass} < baseline {BASELINE_PASS}"
    );
}

#[derive(Default)]
struct Scoreboard {
    /// suite dir -> (pass, fail, skip)
    by_suite: BTreeMap<String, (usize, usize, usize)>,
    failures: Vec<(String, String)>,
    skips: Vec<(String, String)>,
}

impl Scoreboard {
    fn record(&mut self, suite: &str, id: &str, outcome: Outcome) {
        let e = self.by_suite.entry(suite.to_string()).or_default();
        match outcome {
            Outcome::Pass => e.0 += 1,
            Outcome::Fail(why) => {
                e.1 += 1;
                self.failures.push((id.to_string(), why));
            }
            Outcome::Skip(why) => {
                e.2 += 1;
                self.skips.push((id.to_string(), why));
            }
        }
    }
}

enum Outcome {
    Pass,
    Fail(String),
    Skip(String),
}

fn file_iri(path: &FsPath) -> String {
    format!("file://{}", path.display())
}

fn iri_to_path(iri: &str) -> Option<PathBuf> {
    // Fragments / queries never appear in this suite's file references.
    iri.strip_prefix("file://").map(PathBuf::from)
}

fn load(path: &FsPath) -> Result<Graph, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    sparq_shacl::load_turtle_with_base(&text, &file_iri(path))
}

/// Recursively walks `mf:include`s; runs every `mf:entries` entry.
fn walk_manifest(path: &FsPath, root: &FsPath, score: &mut Scoreboard) {
    let path = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            score.record(
                "?",
                &path.display().to_string(),
                Outcome::Fail(format!("canonicalize: {e}")),
            );
            return;
        }
    };
    let suite = path
        .parent()
        .and_then(|d| d.strip_prefix(root).ok())
        .map(|p| {
            if p.as_os_str().is_empty() {
                ".".to_string()
            } else {
                p.display().to_string()
            }
        })
        .unwrap_or_else(|| "?".into());
    let g = match load(&path) {
        Ok(g) => g,
        Err(e) => {
            score.record(
                &suite,
                &path.display().to_string(),
                Outcome::Fail(format!("parse: {e}")),
            );
            return;
        }
    };
    let view = GraphView::new(&g);
    let manifests: Vec<Term> = view
        .triples(None, Some(RDF_TYPE), Some(&iri(&format!("{MF}Manifest"))))
        .into_iter()
        .map(|[s, _, _]| s)
        .collect();
    for m in &manifests {
        for inc in view.objects(m, &format!("{MF}include")) {
            if let Term::NamedNode(n) = &inc {
                if let Some(p) = iri_to_path(n.as_str()) {
                    walk_manifest(&p, root, score);
                }
            }
        }
        for head in view.objects(m, &format!("{MF}entries")) {
            for entry in view.list(&head) {
                let id = match &entry {
                    Term::NamedNode(n) => n
                        .as_str()
                        .strip_prefix(&format!("file://{}/", root.display()))
                        .unwrap_or(n.as_str())
                        .to_string(),
                    other => other.to_string(),
                };
                let outcome = run_entry(&path, &g, &view, &entry)
                    .unwrap_or_else(|e| Outcome::Fail(format!("harness error: {e}")));
                score.record(&suite, &id, outcome);
            }
        }
    }
}

fn run_entry(file: &FsPath, g: &Graph, view: &GraphView, entry: &Term) -> Result<Outcome, String> {
    let types = view.objects(entry, RDF_TYPE);
    let is_validate = types
        .iter()
        .any(|t| matches!(t, Term::NamedNode(n) if n.as_str() == format!("{SHT}Validate")));
    if !is_validate {
        let kind = types
            .first()
            .map(|t| t.to_string())
            .unwrap_or_else(|| "untyped".into());
        return Ok(Outcome::Skip(format!("not sht:Validate ({kind})")));
    }

    let action = view
        .object(entry, &format!("{MF}action"))
        .ok_or("entry has no mf:action")?;
    let graph_of = |pred: &str| -> Result<Option<Graph>, String> {
        match view.object(&action, &format!("{SHT}{pred}")) {
            Some(Term::NamedNode(n)) => {
                let p = iri_to_path(n.as_str()).ok_or_else(|| format!("non-file graph IRI {n}"))?;
                if p == file {
                    Ok(None) // the test document itself — reuse the loaded graph
                } else {
                    load(&p).map(Some)
                }
            }
            other => Err(format!("missing/odd {pred}: {other:?}")),
        }
    };
    let data = graph_of("dataGraph")?;
    let shapes = graph_of("shapesGraph")?;
    let report = validate(data.as_ref().unwrap_or(g), shapes.as_ref().unwrap_or(g));

    // Expected report (inline in the test document).
    let exp_node = view
        .object(entry, &format!("{MF}result"))
        .ok_or("entry has no mf:result")?;
    let exp_conforms = matches!(
        view.object(&exp_node, &format!("{SH}conforms")),
        Some(Term::Literal(l)) if l.value() == "true"
    );
    if exp_conforms != report.conforms {
        return Ok(Outcome::Fail(format!(
            "conforms: expected {exp_conforms}, got {} ({} results)",
            report.conforms,
            report.results.len()
        )));
    }
    let expected: Vec<Expected> = view
        .objects(&exp_node, &format!("{SH}result"))
        .iter()
        .map(|r| Expected::parse(view, r))
        .collect::<Result<_, _>>()?;
    if expected.len() != report.results.len() {
        return Ok(Outcome::Fail(format!(
            "result count: expected {}, got {} — {}",
            expected.len(),
            report.results.len(),
            summarize(&report.results)
        )));
    }
    if !bipartite_match(
        &expected,
        &report.results,
        &mut vec![false; expected.len()],
        0,
    ) {
        return Ok(Outcome::Fail(format!(
            "results do not correspond — got {}",
            summarize(&report.results)
        )));
    }
    Ok(Outcome::Pass)
}

fn summarize(rs: &[ValidationResult]) -> String {
    let items: Vec<String> = rs
        .iter()
        .map(|r| {
            format!(
                "[focus {} comp {} value {}]",
                r.focus_node,
                r.source_component.rsplit('#').next().unwrap_or(""),
                r.value
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into())
            )
        })
        .collect();
    items.join(" ")
}

/// One expected validation result, with absent properties left unconstrained
/// only where the spec leaves them implementation-defined (messages).
struct Expected {
    focus: Option<Term>,
    path: Option<Path>,
    has_path: bool,
    value: Option<Term>,
    has_value: bool,
    severity: Option<String>,
    component: Option<String>,
    source_shape: Option<Term>,
    messages: Vec<Term>,
}

impl Expected {
    fn parse(view: &GraphView, node: &Term) -> Result<Expected, String> {
        let path_term = view.object(node, &format!("{SH}resultPath"));
        let path = match &path_term {
            Some(t) => Some(Path::parse(view, t).map_err(|e| format!("expected path: {e}"))?),
            None => None,
        };
        let value = view.object(node, &format!("{SH}value"));
        Ok(Expected {
            focus: view.object(node, &format!("{SH}focusNode")),
            has_path: path_term.is_some(),
            path,
            has_value: value.is_some(),
            value,
            severity: match view.object(node, &format!("{SH}resultSeverity")) {
                Some(Term::NamedNode(n)) => Some(n.as_str().to_string()),
                _ => None,
            },
            component: match view.object(node, &format!("{SH}sourceConstraintComponent")) {
                Some(Term::NamedNode(n)) => Some(n.as_str().to_string()),
                _ => None,
            },
            source_shape: view.object(node, &format!("{SH}sourceShape")),
            messages: view.objects(node, &format!("{SH}resultMessage")),
        })
    }

    fn matches(&self, a: &ValidationResult) -> bool {
        if let Some(f) = &self.focus {
            if !term_matches(f, &a.focus_node) {
                return false;
            }
        }
        // resultPath: strict — expected-absent requires actual-absent.
        if self.has_path != a.path.is_some() || (self.has_path && self.path != a.path) {
            return false;
        }
        if self.has_value != a.value.is_some() {
            return false;
        }
        if let (Some(e), Some(av)) = (&self.value, &a.value) {
            if !term_matches(e, av) {
                return false;
            }
        }
        if let Some(s) = &self.severity {
            if s != &a.severity {
                return false;
            }
        }
        if let Some(c) = &self.component {
            if c != &a.source_component {
                return false;
            }
        }
        if let Some(s) = &self.source_shape {
            if !term_matches(s, &a.source_shape) {
                return false;
            }
        }
        let actual_messages = a.effective_messages();
        self.messages.iter().all(|m| actual_messages.contains(m))
    }
}

/// Term correspondence across graphs: exact, except blank nodes (no stable
/// cross-graph identity) match any blank node.
fn term_matches(expected: &Term, actual: &Term) -> bool {
    match (expected, actual) {
        (Term::BlankNode(_), Term::BlankNode(_)) => true,
        _ => expected == actual,
    }
}

/// Backtracking perfect matching: every expected result claims a distinct
/// actual result it matches (counts already verified equal).
fn bipartite_match(
    expected: &[Expected],
    actual: &[ValidationResult],
    used: &mut Vec<bool>,
    i: usize,
) -> bool {
    if i == expected.len() {
        return true;
    }
    for (j, a) in actual.iter().enumerate() {
        if !used[j] && expected[i].matches(a) {
            used[j] = true;
            if bipartite_match(expected, actual, used, i + 1) {
                return true;
            }
            used[j] = false;
        }
    }
    false
}

fn iri(s: &str) -> Term {
    Term::NamedNode(oxrdf::NamedNode::new_unchecked(s))
}
