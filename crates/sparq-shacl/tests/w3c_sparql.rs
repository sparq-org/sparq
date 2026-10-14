//! Manifest-driven runner for the W3C SHACL-SPARQL test suite
//! (`data-shapes-test-suite/tests/sparql/...`) — the `node/` and `property/`
//! sections that exercise `sh:sparql` constraints.
//!
//! The suite is fetched by `./fetch-shacl-tests.sh` (gitignored); when absent
//! this test SKIPS itself so `cargo test --workspace` stays green on a fresh
//! checkout. The comparison policy and manifest walking mirror `w3c_core.rs`
//! (kept separate — the core runner pins a calibrated core-only baseline).
//!
//! Scope: the `node/` and `property/` sub-suites (plain `sh:sparql`). The
//! `component/` sub-suite (custom SPARQL-based constraint components, i.e. the
//! `sh:parameter` / `sh:validator` declaration machinery) is walked by the
//! sibling `w3c_sparql_component.rs` runner (sq-wys: it resolves the
//! `owl:imports <http://datashapes.org/dash>` via a vendored excerpt and
//! pre-binds `$PATH`). Most of `pre-binding/` (which probes the FULL pre-binding
//! algebra-rewrite semantics, including rejection of queries that re-bind a
//! pre-bound variable) is still out of scope for this milestone and is not
//! walked (see this crate's open beads — `bd list -l area:sparq-shacl`).

use oxrdf::Term;
use sparq_core::Graph;
use sparq_shacl::view::GraphView;
use sparq_shacl::{validate, Path, ValidationResult};
use std::path::{Path as FsPath, PathBuf};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const SHT: &str = "http://www.w3.org/ns/shacl-test#";
const SH: &str = "http://www.w3.org/ns/shacl#";

/// Pass-count floor for the `sh:sparql` node+property sub-suites at
/// the pinned suite commit (`sq-qap0`). A ratchet: it may only RISE. Mirrors
/// `w3c_core.rs`'s `BASELINE_PASS`. The CI `shacl-conformance` job re-checks it.
/// sq-z1xv8 — the VALUE now lives once in the zero-dependency
/// `sparq-conformance-floors` crate, which `sparq-conformance`'s central
/// `scoreboard::SUITES` reads too, so the enforced floor and the reported floor are
/// ONE `const` and cannot drift (replacing the old textual re-read of this file).
/// Raise it THERE; the measurement narrative stays here.
const SHACL_SPARQL_FLOOR: usize = sparq_conformance_floors::shacl::SPARQL_FLOOR;

fn sparql_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/shacl/data-shapes/data-shapes-test-suite/tests/sparql")
}

#[test]
fn w3c_shacl_sparql_node_and_property() {
    let root = sparql_root();
    if !root.exists() {
        eprintln!(
            "SKIP: W3C SHACL suite not present at {} — run crates/sparq-shacl/fetch-shacl-tests.sh",
            root.display()
        );
        return;
    }
    let mut score = Scoreboard::default();
    // Only the node/ and property/ sub-suites (plain sh:sparql).
    for section in ["node", "property"] {
        let m = root.join(section).join("manifest.ttl");
        if m.exists() {
            walk_manifest(&m, &root, &mut score);
        }
    }

    println!("\nW3C SHACL-SPARQL (node + property) scoreboard");
    for (id, why) in &score.failures {
        println!("  FAIL {id}: {why}");
    }
    for (id, why) in &score.skips {
        println!("  SKIP {id}: {why}");
    }
    println!(
        "pass {} / fail {} / skip {}",
        score.pass, score.fail, score.skip
    );
    // Every node + property entry must pass (these are the sh:sparql cases this
    // milestone implements). A new suite revision adding cases here should be
    // triaged deliberately.
    assert_eq!(
        score.fail, 0,
        "SHACL-SPARQL node/property regressions: {} failing",
        score.fail
    );
    assert!(
        score.pass >= SHACL_SPARQL_FLOOR,
        "SHACL-SPARQL pass count regressed: {} < floor {SHACL_SPARQL_FLOOR}",
        score.pass
    );
}

#[derive(Default)]
struct Scoreboard {
    pass: usize,
    fail: usize,
    skip: usize,
    failures: Vec<(String, String)>,
    skips: Vec<(String, String)>,
}

impl Scoreboard {
    fn record(&mut self, id: &str, outcome: Outcome) {
        match outcome {
            Outcome::Pass => self.pass += 1,
            Outcome::Fail(why) => {
                self.fail += 1;
                self.failures.push((id.to_string(), why));
            }
            Outcome::Skip(why) => {
                self.skip += 1;
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
    iri.strip_prefix("file://").map(PathBuf::from)
}

fn load(path: &FsPath) -> Result<Graph, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    sparq_shacl::load_turtle_with_base(&text, &file_iri(path))
}

fn walk_manifest(path: &FsPath, root: &FsPath, score: &mut Scoreboard) {
    let path = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            score.record(
                &path.display().to_string(),
                Outcome::Fail(format!("canonicalize: {e}")),
            );
            return;
        }
    };
    let g = match load(&path) {
        Ok(g) => g,
        Err(e) => {
            score.record(
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
                score.record(&id, outcome);
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
                    Ok(None)
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

    let exp_node = view
        .object(entry, &format!("{MF}result"))
        .ok_or("entry has no mf:result")?;
    let exp_conforms = matches!(
        view.object(&exp_node, &format!("{SH}conforms")),
        Some(Term::Literal(l)) if l.value() == "true"
    );
    if exp_conforms != report.conforms {
        return Ok(Outcome::Fail(format!(
            "conforms: expected {exp_conforms}, got {} ({} results: {})",
            report.conforms,
            report.results.len(),
            summarize(&report.results)
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
    rs.iter()
        .map(|r| {
            format!(
                "[focus {} comp {} value {} path {}]",
                r.focus_node,
                r.source_component.rsplit('#').next().unwrap_or(""),
                r.value
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
                r.path
                    .as_ref()
                    .map(|p| p.to_turtle())
                    .unwrap_or_else(|| "-".into()),
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

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

fn term_matches(expected: &Term, actual: &Term) -> bool {
    match (expected, actual) {
        (Term::BlankNode(_), Term::BlankNode(_)) => true,
        _ => expected == actual,
    }
}

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
