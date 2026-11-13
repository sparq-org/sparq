//! Manifest-driven runner for the full SHACL 1.2 Core tree at the suite pin.
//! Both feature states compare conforms and result multisets, including stated
//! focus, path, value, severity, component, shape and messages. Constraints that
//! the build does not implement are structural skips, rather than weakened
//! comparisons. Measured pass floors and failure ceilings gate both states.
//! A missing suite checkout skips the runner; fetch-shacl-tests.sh supplies it.

mod support;

use support::Outcome;

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

/// Constraint-component *predicates* (full IRIs) this crate's Core path does NOT
/// implement; an entry whose shapes use any of these is a SKIP rather than a
/// FAIL. `sh:sparql` (SPARQL-based constraints) and `sh:js` (JavaScript
/// constraints) are out of scope for the Core validator; `sh:nodeByExpression` /
/// `sh:expression` are implemented only under `shacl-af` and added per-build in
/// [`unimplemented`].
const UNIMPLEMENTED_CORE: &[&str] = &[
    "http://www.w3.org/ns/shacl#sparql",
    "http://www.w3.org/ns/shacl#js",
    "http://www.w3.org/ns/shacl#jsLibrary",
];

/// The unimplemented-constraint predicate set for the current build. Feature
/// dependent: `sh:nodeByExpression` / `sh:expression` are implemented only with
/// `shacl-af`.
fn unimplemented() -> Vec<String> {
    let mut v: Vec<String> = UNIMPLEMENTED_CORE.iter().map(|s| s.to_string()).collect();
    if cfg!(not(feature = "shacl-af")) {
        v.push(format!("{SH}nodeByExpression"));
        v.push(format!("{SH}expression"));
    }
    v
}

/// Measured at 04ecbb4 (2026-11-12), in both feature states.
const BASELINE_PASS_CORE: usize = 162;
const BASELINE_PASS_AF: usize = 162;
const BASELINE_FAIL_CORE: usize = 0;
const BASELINE_FAIL_AF: usize = 0;

fn baseline_pass() -> usize {
    if cfg!(feature = "shacl-af") {
        BASELINE_PASS_AF
    } else {
        BASELINE_PASS_CORE
    }
}

fn baseline_fail() -> usize {
    if cfg!(feature = "shacl-af") {
        BASELINE_FAIL_AF
    } else {
        BASELINE_FAIL_CORE
    }
}

fn suite_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/shacl/data-shapes/shacl12-test-suite/tests/core")
}

#[test]
fn w3c_shacl12_core_full_suite() {
    let root = suite_root();
    if !root.exists() {
        eprintln!(
            "SKIP: W3C SHACL 1.2 core suite not present at {} — run crates/sparq-shacl/fetch-shacl-tests.sh",
            root.display()
        );
        return;
    }
    let unimpl = unimplemented();
    let mut score = Scoreboard::default();
    walk_manifest(&root.join("manifest.ttl"), &root, &unimpl, &mut score);

    let (mut pass, mut fail, mut skip) = (0usize, 0usize, 0usize);
    println!(
        "\nW3C SHACL 1.2 FULL core suite scoreboard (shacl-af = {})",
        cfg!(feature = "shacl-af")
    );
    println!("{:<24} {:>5} {:>5} {:>5}", "category", "pass", "fail", "skip");
    for (group, (p, f, s)) in &score.by_group {
        println!("{group:<24} {p:>5} {f:>5} {s:>5}");
        pass += p;
        fail += f;
        skip += s;
    }
    println!("{:<24} {pass:>5} {fail:>5} {skip:>5}", "TOTAL");
    if !score.failures.is_empty() {
        println!("\nFAIL detail:");
        for (id, why) in &score.failures {
            println!("  {id}: {why}");
        }
    }
    if !score.skips.is_empty() {
        println!("\nSKIP (out-of-scope constraint surface):");
        for (id, why) in &score.skips {
            println!("  {id}: {why}");
        }
    }
    println!("TOTAL pass {pass} fail {fail} skip {skip}");
    // Two-sided ratchet: pass must not drop, fail must not grow. Together they
    // detect a regression in either direction while staying green at the current
    // (partial) SHACL-1.2 coverage. This is NOT an all-pass assertion — the
    // failing entries are the live gap map (research/shacl12-conformance-gap.md).
    assert!(
        pass >= baseline_pass(),
        "SHACL 1.2 full-core pass count regressed: {pass} < floor {} (fail {fail}, skip {skip})",
        baseline_pass()
    );
    assert!(
        fail <= baseline_fail(),
        "SHACL 1.2 full-core fail count grew: {fail} > ceiling {} — a previously-correct entry broke (pass {pass}, skip {skip})",
        baseline_fail()
    );
}

#[derive(Default)]
struct Scoreboard {
    /// category -> (pass, fail, skip), in a BTreeMap so the print is stable.
    by_group: BTreeMap<String, (usize, usize, usize)>,
    failures: Vec<(String, String)>,
    skips: Vec<(String, String)>,
}

impl Scoreboard {
    fn record(&mut self, group: &str, id: &str, outcome: Outcome) {
        support::earl(id, &outcome);
        let e = self.by_group.entry(group.to_string()).or_default();
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


fn file_iri(path: &FsPath) -> String {
    format!("file://{}", path.display())
}

fn iri_to_path(iri: &str) -> Option<PathBuf> {
    iri.strip_prefix("file://").map(PathBuf::from)
}

fn load(path: &FsPath) -> Result<Graph, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    sparq_shacl::load_turtle_with_base(&text, &file_iri(path))
}

/// The category label for an entry's manifest file: the first path component
/// under `core/` (`node`, `path`, `property`, …), so the scoreboard groups by
/// suite category.
fn category(path: &FsPath, root: &FsPath) -> String {
    path.parent()
        .and_then(|d| d.strip_prefix(root).ok())
        .and_then(|p| p.components().next())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".into())
}

/// Recursively walks `mf:include`s; runs every `sht:Validate` entry.
fn walk_manifest(path: &FsPath, root: &FsPath, unimpl: &[String], score: &mut Scoreboard) {
    let path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return,
    };
    let group = category(&path, root);
    let g = match load(&path) {
        Ok(g) => g,
        Err(e) => {
            score.record(
                &group,
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
                    walk_manifest(&p, root, unimpl, score);
                }
            }
        }
        for head in view.objects(m, &format!("{MF}entries")) {
            for entry in view.list(&head) {
                let id = match &entry {
                    Term::NamedNode(n) => n.as_str().to_string(),
                    other => other.to_string(),
                };
                let outcome = run_entry(&path, &g, &view, &entry, unimpl)
                    .unwrap_or_else(|e| Outcome::Fail(format!("harness error: {e}")));
                score.record(&group, &id, outcome);
            }
        }
    }
}

fn run_entry(
    file: &FsPath,
    g: &Graph,
    view: &GraphView,
    entry: &Term,
    unimpl: &[String],
) -> Result<Outcome, String> {
    let is_validate = view
        .objects(entry, RDF_TYPE)
        .iter()
        .any(|t| matches!(t, Term::NamedNode(n) if n.as_str() == format!("{SHT}Validate")));
    if !is_validate {
        return Ok(Outcome::Skip("not sht:Validate".into()));
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
    let shapes_graph = shapes.as_ref().unwrap_or(g);

    // SKIP up front when the shapes use a constraint FORM this build does not
    // implement: the report would necessarily mismatch, but that is an absence of
    // a feature, not a regression. Structural (not outcome-driven) so a wrong
    // report on an IMPLEMENTED constraint can never be laundered into a SKIP.
    if let Some(why) = unimplemented_form(shapes_graph, unimpl) {
        return Ok(Outcome::Skip(why));
    }

    let report = validate(data.as_ref().unwrap_or(g), shapes_graph);

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

/// The reason (if any) the shapes graph uses a constraint **predicate** this
/// build does not implement — identifying an entry to SKIP. Returns `None` when
/// every constraint the shapes use is implemented (so the entry is compared
/// strictly).
fn unimplemented_form(shapes: &Graph, unimpl: &[String]) -> Option<String> {
    let view = GraphView::new(shapes);
    if let Some(p) = unimpl.iter().find(|p| !view.subjects_of(p).is_empty()) {
        return Some(format!("shapes use unimplemented constraint <{p}>"));
    }
    None
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

/// One expected validation result. Absent properties are left unconstrained.
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

/// Term correspondence across graphs: exact, except blank nodes match any blank
/// node (they have no stable cross-graph identity).
fn term_matches(expected: &Term, actual: &Term) -> bool {
    match (expected, actual) {
        (Term::BlankNode(_), Term::BlankNode(_)) => true,
        _ => expected == actual,
    }
}

/// Backtracking perfect matching: every expected result claims a distinct actual
/// result it matches (counts already verified equal).
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
