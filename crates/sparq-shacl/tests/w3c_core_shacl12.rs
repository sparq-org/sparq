//! (sq-yca1) Manifest-driven runner for the **SHACL 1.2** core test
//! suite's `node/` `sht:Validate` entries
//! (`shacl12-test-suite/tests/core/node/...`).
//!
//! `w3c_core.rs` walks the *older* `data-shapes-test-suite/tests/core` tree. The
//! newer `shacl12-test-suite/tests/core/node` tree additionally carries a
//! `nodeByExpression-001` `sht:Validate` entry (the constant-IRI form of
//! `sh:nodeByExpression`, implemented under the `shacl-af` feature in sq-3w6n)
//! **plus** the SHACL-1.2-only core constraints `sh:memberShape`,
//! `sh:uniqueMembers`, `sh:{max,min}ListLength`, `sh:uniqueValuesFor`, the
//! `sh:closed sh:ByTypes` close-by-types mode, and the disjunctive list spellings
//! of `sh:datatype` / `sh:nodeKind`. (sq-vg3y) all of those are now
//! implemented on the Core path and compared strictly; the only thing this
//! harness still SKIPs is `sh:nodeByExpression` when `shacl-af` is off.
//!
//! ## SKIP-tolerance (honest floor)
//!
//! An entry is **SKIP**, not FAIL, exactly when its shapes graph uses a
//! constraint-component *predicate* with no counterpart in this build
//! ([`unimplemented`]). After sq-vg3y that is ONLY `sh:nodeByExpression`, and ONLY
//! when `shacl-af` is off.
//!
//! Any entry whose shapes use **only** implemented constraints is compared
//! strictly — a mismatch there is a FAIL, so the harness can never silently mask
//! a regression in an implemented constraint.
//!
//! This is deliberately a *structural* SKIP (vocabulary) and not an
//! outcome-driven one: an entry is classed up front by the predicates it
//! exercises, so a wrong report on an *implemented* constraint cannot be laundered
//! into a SKIP.
//!
//! The floor auto-grows as the SKIP list shrinks (bump [`BASELINE_PASS_CORE`] /
//! [`BASELINE_PASS_AF`] then).
//!
//! ## Two gates
//!
//!   * **Suite gate** — the suite is fetched by `./fetch-shacl-tests.sh` into
//!     `tests/shacl/` (gitignored). When absent the test SKIPS itself so
//!     `cargo test --workspace` stays green on a fresh checkout.
//!   * **Feature gate** — this file is *not* `#[cfg]`-gated as a whole: it runs in
//!     **both** feature states. With `shacl-af` OFF, `sh:nodeByExpression` is
//!     unimplemented, so `nodeByExpression-001` is a SKIP and the pass-floor is
//!     [`BASELINE_PASS_CORE`]. With `shacl-af` ON, that entry is PASS and the
//!     floor rises to [`BASELINE_PASS_AF`].
//!
//! The report-comparison policy mirrors `w3c_core.rs`: `sh:conforms` must match,
//! and the result multisets must correspond 1:1 where each expected result's
//! stated `sh:focusNode` / `sh:resultPath` / `sh:value` / `sh:resultSeverity` /
//! `sh:sourceConstraintComponent` / `sh:sourceShape` / `sh:resultMessage` agree
//! (blank nodes match any blank node).

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

/// Constraint-component *predicates* (relative to `sh:`) this crate does NOT
/// implement. An entry whose shapes use any of these is recorded as SKIP rather
/// than FAIL.
///
/// (sq-vg3y) The SHACL-1.2-only core/node constraints
/// (`sh:memberShape`, `sh:uniqueMembers`, `sh:{max,min}ListLength`,
/// `sh:uniqueValuesFor`) are now implemented on the Core path, so this set is
/// EMPTY. The only feature-dependent absence left is `sh:nodeByExpression`
/// (implemented under `shacl-af` only) — added to the per-build set in
/// [`unimplemented`] when that feature is off.
const UNIMPLEMENTED_CORE: &[&str] = &[];

/// The unimplemented-constraint predicate set for the current build, as full
/// `sh:`-prefixed IRIs. Feature-dependent: `sh:nodeByExpression` is implemented
/// only with `shacl-af`.
fn unimplemented() -> Vec<String> {
    let mut v: Vec<String> = UNIMPLEMENTED_CORE
        .iter()
        .map(|local| format!("{SH}{local}"))
        .collect();
    if cfg!(not(feature = "shacl-af")) {
        v.push(format!("{SH}nodeByExpression"));
    }
    v
}

/// Pass-floor with the `shacl-af` feature OFF: the implemented core/node
/// `sht:Validate` entries at the pinned suite commit, excluding
/// `nodeByExpression-001` (which is then a SKIP). A regression below this fails
/// the build; raising it is a deliberate bump.
///
/// (sq-vg3y) Bumped 32 → 44: the twelve formerly-SKIPed SHACL-1.2-only
/// core/node entries (`closed-003/004`, `datatype-003`, `nodeKind-002`,
/// `memberShape-001`, `uniqueMembers-001`, `{max,min}ListLength-001`,
/// `uniqueValuesFor-001..004`) are now implemented and PASS.
const BASELINE_PASS_CORE: usize = 44;

/// Pass-floor with the `shacl-af` feature ON: [`BASELINE_PASS_CORE`] plus the
/// `nodeByExpression-001` entry, which the SHACL-AF `sh:nodeByExpression`
/// implementation (sq-3w6n) now validates end to end.
const BASELINE_PASS_AF: usize = BASELINE_PASS_CORE + 1;

fn baseline_pass() -> usize {
    if cfg!(feature = "shacl-af") {
        BASELINE_PASS_AF
    } else {
        BASELINE_PASS_CORE
    }
}

fn suite_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/shacl/data-shapes/shacl12-test-suite/tests/core/node")
}

#[test]
fn w3c_shacl12_core_node_suite() {
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
    walk_manifest(&root.join("manifest.ttl"), &unimpl, &mut score);

    let (mut pass, mut fail, mut skip) = (0, 0, 0);
    println!(
        "\nW3C SHACL 1.2 core/node suite scoreboard (shacl-af = {})",
        cfg!(feature = "shacl-af")
    );
    for (id, outcome) in &score.outcomes {
        match outcome {
            Outcome::Pass => {
                pass += 1;
                println!("  PASS {id}");
            }
            Outcome::Fail(why) => {
                fail += 1;
                println!("  FAIL {id}: {why}");
            }
            Outcome::Skip(why) => {
                skip += 1;
                println!("  SKIP {id}: {why}");
            }
        }
    }
    println!("TOTAL pass {pass} fail {fail} skip {skip}");
    assert_eq!(
        fail, 0,
        "SHACL 1.2 core/node: {fail} entries using only IMPLEMENTED constraints produced the wrong report"
    );
    assert!(
        pass >= baseline_pass(),
        "SHACL 1.2 core/node pass count regressed: {pass} < baseline {} (skipped {skip})",
        baseline_pass()
    );
}

#[derive(Default)]
struct Scoreboard {
    /// id -> outcome, in a BTreeMap so the scoreboard print is stable.
    outcomes: BTreeMap<String, Outcome>,
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

/// Recursively walks `mf:include`s; runs every `sht:Validate` entry.
fn walk_manifest(path: &FsPath, unimpl: &[String], score: &mut Scoreboard) {
    let path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return,
    };
    let g = match load(&path) {
        Ok(g) => g,
        Err(e) => {
            score.outcomes.insert(
                path.display().to_string(),
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
                    walk_manifest(&p, unimpl, score);
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
                score.outcomes.insert(id, outcome);
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
    // report on an IMPLEMENTED constraint form can never be laundered into a SKIP.
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

/// The reason (if any) the shapes graph uses a constraint **form** this build
/// does not implement — identifying an entry to SKIP.
///
/// (sq-vg3y) This now reduces to a single case: an
/// unimplemented-constraint *predicate* (the [`unimplemented`] set), which after
/// sq-vg3y contains ONLY `sh:nodeByExpression` and ONLY when `shacl-af` is off.
/// The previously-SKIPed *forms* of implemented predicates — `sh:closed
/// sh:ByTypes`, and the disjunctive list spellings of `sh:datatype` /
/// `sh:nodeKind` — are now implemented on the Core path and so compared strictly.
///
/// Returns `None` when every constraint the shapes use is in a form the crate
/// implements (so the entry is compared strictly).
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
