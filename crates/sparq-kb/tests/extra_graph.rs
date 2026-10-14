//! `--extra-graph` loader tests — the FO-KM benchmark seam (epic sq-mztg8, Metric 1).
//!
//! 🤖 SPARQ agent. Proves the two load-bearing invariants of the
//! `load_pkg_with_extra` / `load_pkg_closed_with_extra` helpers the `pkg-query
//! --extra-graph` flag uses:
//!
//!   1. an extra graph's triples are VISIBLE to a query alongside the PKG, and
//!   2. an extra graph's `rdfs:subClassOf` axioms PARTICIPATE in `--close owl-rl`, so
//!      a foundational-ontology overlay's typed facts are ENTAILED (the FO arm answers
//!      a type-hierarchy question the no-FO arm cannot).
//!
//! Run:
//!   cargo test -p sparq-kb --features query --test extra_graph -- --nocapture
//!   cargo test -p sparq-kb --features close --test extra_graph -- --nocapture
#![cfg(feature = "query")]

use oxrdf::Term;
use sparq_kb::query::{ask_pkg, load_pkg, load_pkg_with_extra};

/// A tiny self-contained overlay (declares its own prefixes, as the loader requires):
/// it adds one brand-new triple AND types `pkg:Task` under a foundational category so
/// the closure can entail the FO type onto every Task instance.
const TINY_OVERLAY: &str = r#"
@prefix pkg:   <https://sparq.dev/ns/pkg#> .
@prefix ex:    <https://example.test/fo#> .
@prefix rdfs:  <http://www.w3.org/2000/01/rdf-schema#> .
@prefix owl:   <http://www.w3.org/2002/07/owl#> .

ex:Process a owl:Class .
ex:marker  ex:says "extra-graph-visible" .
pkg:Task rdfs:subClassOf ex:Process .
"#;

/// Count helper: pull the single integer COUNT(*) value out of a one-row result.
fn count_of(r: &sparq_engine::QueryResult) -> i64 {
    match r.rows.first().and_then(|row| row.first()) {
        Some(Some(Term::Literal(l))) => l.value().parse::<i64>().unwrap_or(-1),
        _ => -1,
    }
}

/// Invariant 1: a triple that exists ONLY in the extra graph is visible to a query over
/// the combined store — and is absent from the plain PKG (so the extra graph is what
/// made it visible, not the base data).
#[test]
fn extra_graph_triples_are_visible_to_a_query() {
    let marker = r#"
PREFIX ex: <https://example.test/fo#>
SELECT ?v WHERE { ex:marker ex:says ?v }"#;

    // Absent from the plain PKG.
    let base = load_pkg().expect("PKG loads");
    let r0 = ask_pkg(&base, marker).expect("marker query runs over base");
    assert!(
        r0.rows.is_empty(),
        "the marker triple must NOT be in the plain PKG; got {} row(s)",
        r0.rows.len()
    );

    // Present once the overlay is loaded alongside.
    let g = load_pkg_with_extra(&[TINY_OVERLAY]).expect("PKG + overlay loads");
    let r = ask_pkg(&g, marker).expect("marker query runs over combined");
    assert_eq!(
        r.rows.len(),
        1,
        "the extra graph's marker triple must be visible"
    );
    match &r.rows[0][0] {
        Some(Term::Literal(l)) => assert_eq!(l.value(), "extra-graph-visible"),
        other => panic!("unexpected marker value: {other:?}"),
    }
}

/// Empty `extra_docs` is identical to `load_pkg` — the helper is a strict superset.
#[test]
fn no_extra_graph_matches_plain_load() {
    let a = load_pkg().expect("plain");
    let b = load_pkg_with_extra(&[]).expect("with empty extra");
    let q =
        "PREFIX pkg: <https://sparq.dev/ns/pkg#> SELECT (COUNT(*) AS ?n) WHERE { ?s a pkg:Task }";
    let na = count_of(&ask_pkg(&a, q).expect("a"));
    let nb = count_of(&ask_pkg(&b, q).expect("b"));
    assert!(na > 0, "the PKG must hold Tasks");
    assert_eq!(na, nb, "empty extra_docs must not change the result");
}

/// Invariant 2 (closure): the overlay's `rdfs:subClassOf` axiom participates in
/// `--close owl-rl`, so every `pkg:Task` is ENTAILED to be the foundational type — a
/// query over the FO category returns the Tasks the no-FO graph could not. Only built
/// with `--features close`.
#[cfg(feature = "close")]
#[test]
fn extra_graph_subclassof_participates_in_owl_rl_closure() {
    use sparq_kb::query::close::{load_pkg_closed_with_extra, Profile};

    // The FO-typed query: how many ex:Process individuals are there?
    let fo_query = r#"
PREFIX ex: <https://example.test/fo#>
SELECT (COUNT(DISTINCT ?x) AS ?n) WHERE { ?x a ex:Process }"#;
    // The asserted-Task baseline to compare against.
    let task_query =
        "PREFIX pkg: <https://sparq.dev/ns/pkg#> SELECT (COUNT(DISTINCT ?x) AS ?n) WHERE { ?x a pkg:Task }";

    // WITHOUT the overlay (no-FO arm), the FO type is not entailed even under closure.
    let (no_fo, _) = load_pkg_closed_with_extra(Profile::OwlRl, &[]).expect("closed no-FO");
    assert_eq!(
        count_of(&ask_pkg(&no_fo, fo_query).expect("fo over no-FO")),
        0,
        "the no-FO arm must NOT entail the FO type (the discrimination the benchmark needs)"
    );

    // WITH the overlay + closure, ex:Process is entailed onto every Task.
    let (fo, entailed) =
        load_pkg_closed_with_extra(Profile::OwlRl, &[TINY_OVERLAY]).expect("closed FO arm");
    assert!(entailed > 0, "OWL-RL closure must add entailed triples");
    let n_process = count_of(&ask_pkg(&fo, fo_query).expect("fo over FO arm"));
    let n_tasks = count_of(&ask_pkg(&fo, task_query).expect("tasks"));
    assert!(n_tasks > 0, "the PKG must hold Tasks");
    assert_eq!(
        n_process, n_tasks,
        "every pkg:Task must be entailed as the FO type via the overlay's subClassOf + closure"
    );
}
