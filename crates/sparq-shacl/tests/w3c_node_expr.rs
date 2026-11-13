//! Manifest-driven W3C SHACL 1.2 node-expression evaluation in both feature states.
//!
//! Drives the production fallible evaluator. Explicit `sht:Failure` results
//! require an error, distinct from a successful empty output list. Output order
//! and duplicates matter unless the entry requests `sht:ignoreOrder true`, in
//! which case the runner compares multisets. Unsupported expressions count as
//! failures rather than skipped evaluation entries.
//!
//! Validation entries belong to the separate constraint runner. Missing suite
//! checkouts skip the runner; every fetched evaluation entry is attempted.

mod support;

use support::Outcome;

use oxrdf::Term;
use sparq_core::Graph;
use sparq_shacl::eval_node_expression_result;
use sparq_shacl::view::GraphView;
use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const SHT: &str = "http://www.w3.org/ns/shacl-test#";

/// Pass-rate floor: the number of `sht:EvalNodeExpr` entries the crate's REAL
/// node-expression evaluation gets right at the pinned suite commit
/// (`fetch-shacl-tests.sh` PIN). PASS = the result matches by sequence, or by
/// multiset when the entry sets `sht:ignoreOrder true`. A regression below this
/// floor fails the build; raising it is a deliberate bump.
///
/// Measured at 04ecbb4 (2026-11-12) in both feature states. Four decimal
/// lexical-form differences retain their failing EARL assertions.
const BASELINE_PASS: usize = 272;
const BASELINE_FAIL: usize = 4;

fn suite_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/shacl/data-shapes/shacl12-test-suite/tests/node-expr")
}

#[test]
fn w3c_shacl_node_expr_suite() {
    let root = suite_root();
    if !root.exists() {
        eprintln!(
            "SKIP: W3C SHACL node-expr suite not present at {} — run crates/sparq-shacl/fetch-shacl-tests.sh",
            root.display()
        );
        return;
    }
    let mut score = Scoreboard::default();
    walk_manifest(&root.join("manifest.ttl"), &root, &mut score);

    let (mut pass, mut fail, mut skip) = (0, 0, 0);
    println!("\nW3C SHACL node-expr suite scoreboard");
    println!("{:<28} {:>5} {:>5} {:>5}", "group", "pass", "fail", "skip");
    for (group, c) in &score.by_group {
        println!("{group:<28} {:>5} {:>5} {:>5}", c.pass, c.fail, c.skip);
        pass += c.pass;
        fail += c.fail;
        skip += c.skip;
    }
    println!("{:<28} {pass:>5} {fail:>5} {skip:>5}", "TOTAL");
    if !score.failures.is_empty() {
        println!("\nfailing tests:");
        for (id, why) in &score.failures {
            println!("  {id}: {why}");
        }
    }
    if !score.skips.is_empty() {
        println!("\nskipped (unsupported form / harness):");
        for (id, why) in &score.skips {
            println!("  {id}: {why}");
        }
    }
    assert!(
        fail <= BASELINE_FAIL,
        "SHACL node-expr: {fail} supported-form entries produced the wrong result set (real eval)"
    );
    assert!(
        pass >= BASELINE_PASS,
        "SHACL node-expr pass count regressed: {pass} < baseline {BASELINE_PASS} (skipped {skip})"
    );
}

#[derive(Default)]
struct Counts {
    pass: usize,
    fail: usize,
    skip: usize,
}

#[derive(Default)]
struct Scoreboard {
    by_group: BTreeMap<String, Counts>,
    failures: Vec<(String, String)>,
    skips: Vec<(String, String)>,
}

impl Scoreboard {
    fn record(&mut self, group: &str, id: &str, outcome: Outcome) {
        support::earl(id, &outcome);
        let e = self.by_group.entry(group.to_string()).or_default();
        match outcome {
            Outcome::Pass => e.pass += 1,
            Outcome::Fail(why) => {
                e.fail += 1;
                self.failures.push((id.to_string(), why));
            }
            Outcome::Skip(why) => {
                e.skip += 1;
                self.skips.push((id.to_string(), why));
            }
        }
    }
}


fn file_iri(path: &FsPath) -> String {
    format!("file://{}", path.display())
}

fn load(path: &FsPath) -> Result<Graph, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    sparq_shacl::load_turtle_with_base(&text, &file_iri(path))
}

/// Recursively walks `mf:include`s; runs every `sht:EvalNodeExpr` entry.
fn walk_manifest(path: &FsPath, root: &FsPath, score: &mut Scoreboard) {
    let path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return,
    };
    let group = path
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
                if let Some(p) = n.as_str().strip_prefix("file://") {
                    walk_manifest(FsPath::new(p), root, score);
                }
            }
        }
        for head in view.objects(m, &format!("{MF}entries")) {
            for entry in view.list(&head) {
                let id = match &entry {
                    Term::NamedNode(n) => n.as_str().to_string(),
                    other => other.to_string(),
                };
                let outcome = run_entry(&g, &view, &entry);
                score.record(&group, &id, outcome);
            }
        }
    }
}

fn run_entry(g: &Graph, view: &GraphView, entry: &Term) -> Outcome {
    let is_eval = view
        .objects(entry, RDF_TYPE)
        .iter()
        .any(|t| matches!(t, Term::NamedNode(n) if n.as_str() == format!("{SHT}EvalNodeExpr")));
    if !is_eval {
        return Outcome::Skip("not sht:EvalNodeExpr".into());
    }
    let Some(action) = view.object(entry, &format!("{MF}action")) else {
        return Outcome::Skip("no mf:action".into());
    };
    let Some(expr) = view.object(&action, &format!("{SHT}nodeExpr")) else {
        return Outcome::Skip("no sht:nodeExpr".into());
    };
    // The focus node: sht:focusNode on the action, else (when the expression
    // supplies its own focus) a placeholder — these entries set their own focus.
    let focus = view
        .object(&action, &format!("{SHT}focusNode"))
        .unwrap_or_else(|| iri("urn:x-sparq:no-focus"));

    // (sq-u5rxj) Build the caller-supplied node-expression variable
    // scope from the suite's `sht:scope-<name>` action bindings. A `shnex:var
    // "<name>"` (other than `"focusNode"`) now resolves against this scope through
    // the crate's real evaluation, rather than being recorded as a divergence.
    let scope = harness_scope(view, &action);

    // Drive the crate's parse + fallible evaluation. The test graph is both the data
    // and the shapes graph (the suite declares filter shapes inline).
    let actual = eval_node_expression_result(g, g, &expr, &focus, &scope);
    let Some(expected_term) = view.object(entry, &format!("{MF}result")) else {
        return Outcome::Fail("no mf:result".into());
    };
    if expected_term == iri(&format!("{SHT}Failure")) {
        return match actual {
            Err(_) => Outcome::Pass,
            Ok(nodes) => Outcome::Fail(format!("expected evaluation failure, got {nodes:?}")),
        };
    }
    let actual = match actual {
        Ok(nodes) => nodes,
        Err(error) => return Outcome::Fail(format!("evaluation failure: {error}")),
    };
    let expected = view.list(&expected_term);
    // The suite README: output order must match `mf:result` exactly unless the
    // action sets `sht:ignoreOrder true`, which allows any order with the same
    // cardinalities.
    let ignore_order = matches!(
        view.object(&action, &format!("{SHT}ignoreOrder")),
        Some(Term::Literal(l)) if l.value() == "true"
    );
    if seq_eq(&actual, &expected) || (ignore_order && multiset_eq(&actual, &expected)) {
        Outcome::Pass
    } else if multiset_eq(&actual, &expected) {
        Outcome::Fail(format!(
            "same nodes in a different order, and the entry does not set sht:ignoreOrder: expected {expected:?}, got {actual:?}"
        ))
    } else {
        Outcome::Fail(format!("expected {expected:?}, got {actual:?}"))
    }
}

/// (sq-u5rxj) Builds the caller-supplied node-expression variable scope
/// from the suite's `sht:scope-<name>` action bindings: each
/// `[action] sht:scope-<name> <value>` becomes `name -> { value }`. A `shnex:var
/// "<name>"` resolves against this map (the crate threads it through
/// `eval_node_expression_with_scope`). Empty when the action declares no scope.
fn harness_scope(view: &GraphView, action: &Term) -> sparq_shacl::Scope {
    let mut scope = sparq_shacl::Scope::default();
    let prefix = format!("{SHT}scope-");
    for (p, o) in view.predicate_objects(action) {
        if let Term::NamedNode(pred) = &p {
            if let Some(name) = pred.as_str().strip_prefix(&prefix) {
                scope.entry(name.to_string()).or_default().push(o);
            }
        }
    }
    scope
}

/// Sequence equality: same terms, same order, duplicates significant.
fn seq_eq(a: &[Term], b: &[Term]) -> bool {
    a == b
}

/// Multiset equality: same terms with the same multiplicities, order ignored.
fn multiset_eq(a: &[Term], b: &[Term]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for t in a {
        *counts.entry(key(t)).or_default() += 1;
    }
    for t in b {
        *counts.entry(key(t)).or_default() -= 1;
    }
    counts.values().all(|&c| c == 0)
}

/// A stable string key for multiset bookkeeping (term Debug is stable per kind).
fn key(t: &Term) -> String {
    format!("{:?}", t)
}

fn iri(s: &str) -> Term {
    Term::NamedNode(oxrdf::NamedNode::new_unchecked(s))
}
