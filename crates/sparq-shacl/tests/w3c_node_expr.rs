//! (sq-1m0n) Manifest-driven runner for the W3C SHACL **node
//! expression** test suite (`sht:EvalNodeExpr` entries in
//! `w3c/data-shapes`, `shacl12-test-suite/tests/node-expr/...`).
//!
//! These tests directly evaluate a node expression against a focus node and
//! compare the resulting node set to `mf:result`. They are the W3C conformance
//! surface for the SHACL-AF node-expression algebra implemented in
//! `sparq-shacl`'s `shacl-af` feature.
//!
//! ## Drives the REAL crate evaluation (sq-mue75)
//!
//! (sq-mue75) This runner drives the crate's own node-expression
//! parse + evaluation through the public, feature-gated
//! [`sparq_shacl::eval_node_expression`] seam — it does NOT re-implement the
//! algebra in the harness. The crate parser now accepts both the SHACL 1.2
//! `shnex:` and the SHACL-AF `sh:` spellings of the shared algebra (path-values /
//! nodes / filter-shape / intersection / union), so the suite's `shnex:`-spelled
//! expressions exercise the production code path. A previous version of this file
//! inlined a second copy of the algebra and compared order-blind; that was a
//! fidelity gap (it could pass while the real code was wrong), now closed.
//!
//! ## Order-aware comparison
//!
//! `mf:result` is an `rdf:list`, an ORDERED sequence, and the suite README makes
//! order significant: the output must equal `mf:result` as a sequence (order and
//! duplicates) unless the action sets `sht:ignoreOrder true`, in which case any
//! order with the same cardinalities passes. A result with the right nodes in the
//! wrong order, on an entry without `sht:ignoreOrder`, is a FAIL.
//!
//! ## Two gates, both opt-in
//!
//!   * **Feature gate** — the whole runner is `#[cfg(feature = "shacl-af")]`; with
//!     the feature off the file compiles to nothing (the node-expression public
//!     API it drives is itself feature-gated).
//!   * **Suite gate** — the suite is fetched by `./fetch-shacl-tests.sh` into
//!     `tests/shacl/` (gitignored). When absent the test SKIPS itself so
//!     `cargo test --workspace` stays green on a fresh checkout.
//!
//! ## Scope (honest)
//!
//! The runner covers the **implemented algebra** end-to-end. (sq-u5rxj)
//! The suite's `shnex:var "<name>"` entries that resolve a `sht:scope-<name>`
//! action binding are now driven through the crate's real evaluation: the harness
//! builds a [`sparq_shacl::Scope`] from those bindings and passes it to
//! [`sparq_shacl::eval_node_expression_with_scope`], so `var-bound` PASSES (the
//! previous `XFAIL_VAR_SCOPE` divergence is closed). Any entry whose form the
//! crate cannot parse is a SKIP here and a failure in the EARL report. The gate
//! is an honest floor over exactly what the production path evaluates.

#![cfg(feature = "shacl-af")]

mod support;

use support::Outcome;

use oxrdf::Term;
use sparq_core::Graph;
use sparq_shacl::eval_node_expression_with_scope;
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
/// Measured at 04ecbb4 (2026-11-12). Evaluation gaps are tracked separately
/// from unsupported forms (skips); EARL reports both as failed assertions.
const BASELINE_PASS: usize = 123;
const BASELINE_FAIL: usize = 9;

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

    // Drive the crate's REAL parse + evaluation. The test graph is both the data
    // and the shapes graph (the suite declares filter shapes inline).
    let actual = match eval_node_expression_with_scope(g, g, &expr, &focus, &scope) {
        Some(v) => v,
        None => return Outcome::Skip("crate could not parse the node expression".into()),
    };

    let expected = match view.object(entry, &format!("{MF}result")) {
        Some(list) => view.list(&list),
        None => return Outcome::Skip("no mf:result".into()),
    };
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
