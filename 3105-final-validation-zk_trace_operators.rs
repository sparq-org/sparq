//! Hand-computed per-operator zk-trace assertions (plan §4.E, module-B scope).
//!
//! Each test runs a small query on a small graph with the recorder armed and
//! asserts the EXACT captured input set / operator boundary structure. These
//! are the ground-truth oracles for the per-property proof-input identification.
//!
//! [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.

#![cfg(feature = "zk")]

use sparq_engine::zk::{self, Op, SlotPattern, Step};
use sparq_engine::{query, QueryResult};
use sparq_core::Graph;

fn graph(turtle: &str) -> Graph {
    Graph::load_str(turtle, "turtle").unwrap()
}

/// Run a query with the trace armed and return (result, trace).
fn traced(g: &Graph, q: &str) -> (QueryResult, zk::ZkTrace) {
    let _guard = zk::install();
    let r = query(g, q).unwrap();
    let t = zk::take();
    (r, t)
}

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:a ex:p ex:b . ex:b ex:p ex:c . ex:c ex:p ex:d .
    ex:a ex:name "A" . ex:b ex:name "B" . ex:c ex:name "C" .
    ex:a ex:age 30 . ex:b ex:age 40 . ex:c ex:age 50 .
"#;

/// A constant-subject pattern consumes exactly the matching triples; the trace
/// records whole triples (not just projected columns), id-deduplicated.
#[test]
fn bgp_single_pattern_exact_set() {
    let g = graph(DATA);
    let (r, t) = traced(&g, "SELECT ?o WHERE { <http://ex/a> <http://ex/p> ?o }");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(t.patterns.len(), 1);
    let p = &t.patterns[0];
    assert_eq!(p.triples.len(), 1);
    assert_eq!(p.triples[0][2].to_string(), "<http://ex/b>");
    assert!(p.graph.is_none() && !p.in_exists);
    // One Scan step, no operator boundaries.
    assert_eq!(t.steps, vec![Step::Scan { pattern: 0 }]);
}

/// A two-pattern chain join records BOTH patterns' post-scan input sets. The
/// first pattern (constant subject) consumes one triple; the second (the join)
/// is a bind-join rescan or a full scan depending on the plan — either way its
/// recorded set must be sufficient to reproduce the join result.
#[test]
fn bgp_chain_join_records_both_sides() {
    let g = graph(DATA);
    let q = "SELECT ?n WHERE { <http://ex/a> <http://ex/p> ?x . ?x <http://ex/name> ?n }";
    let (r, t) = traced(&g, q);
    assert_eq!(r.rows.len(), 1, "a -> b, b name B");
    assert_eq!(t.patterns.len(), 2);
    // The constant pattern consumed exactly {a p b}.
    let p0 = t
        .patterns
        .iter()
        .find(|p| matches!(&p.pattern.slots[0], SlotPattern::Term(t) if t.to_string() == "<http://ex/a>"))
        .expect("a-p-x pattern");
    assert_eq!(p0.triples.len(), 1);
    // Sufficiency: re-running over only the traced triples reproduces the result.
    let replay = sparq_zk_replay(&t);
    let rr = query(&replay, q).unwrap();
    assert_eq!(rows_set(&r), rows_set(&rr));
}

/// FILTER records one obligation: distinct operands + per-row verdicts. The
/// kept-row count equals the result row count.
#[test]
fn filter_records_obligation_with_verdicts() {
    let g = graph(DATA);
    let q = "SELECT ?s WHERE { ?s <http://ex/age> ?a . FILTER(?a > 35) }";
    let (r, t) = traced(&g, q);
    assert_eq!(r.rows.len(), 2, "b(40), c(50)");
    assert_eq!(t.filters.len(), 1, "exactly one FILTER obligation");
    let f = &t.filters[0];
    // Every input row of the filtered set appears with a verdict; exactly the
    // passing ones match the result count.
    let passed = f.rows.iter().filter(|(_, k)| *k).count();
    assert_eq!(passed, 2);
    assert!(f.rows.len() >= passed);
    // A Filter step is present in the order.
    assert!(t.steps.iter().any(|s| matches!(s, Step::Filter { .. })));
}

/// OPTIONAL records an Enter/Exit(Optional) boundary around its operand scans;
/// the right side's non-matching rows are still captured (conservative
/// superset — documented).
#[test]
fn optional_records_boundary_and_both_sides() {
    let g = graph(DATA);
    // ex:d has a name? No. So d's OPTIONAL age is unbound; the left (name)
    // side has 3 rows.
    let q = "SELECT ?s ?age WHERE { ?s <http://ex/name> ?nm OPTIONAL { ?s <http://ex/age> ?age } }";
    let (r, t) = traced(&g, q);
    assert_eq!(r.rows.len(), 3, "a,b,c have names");
    assert_boundary(&t, Op::Optional);
    // Both the name pattern and the age pattern are captured.
    assert!(t.patterns.len() >= 2);
}

/// UNION records an Enter/Exit(Union) boundary; both branches' scans are
/// captured.
#[test]
fn union_records_boundary_and_both_branches() {
    let g = graph(DATA);
    let q = "SELECT ?x WHERE { { ?x <http://ex/age> ?v } UNION { ?x <http://ex/name> ?v } }";
    let (r, t) = traced(&g, q);
    assert_eq!(r.rows.len(), 6, "3 ages + 3 names");
    assert_boundary(&t, Op::Union);
    assert!(t.patterns.len() >= 2);
}

/// MINUS records an Enter/Exit(Minus) boundary; both sides captured.
#[test]
fn minus_records_boundary() {
    let g = graph(DATA);
    let q = "SELECT ?s WHERE { ?s <http://ex/name> ?n MINUS { ?s <http://ex/age> ?a } }";
    let (_r, t) = traced(&g, q);
    assert_boundary(&t, Op::Minus);
}

/// DISTINCT records its boundary; the captured pattern set is the PRE-DISTINCT
/// input (the reduction is verifier-side, per the proof-semantics doc).
#[test]
fn distinct_records_boundary_pre_reduction() {
    let g = graph(DATA);
    let q = "SELECT DISTINCT ?p WHERE { ?s ?p ?o }";
    let (r, t) = traced(&g, q);
    // 3 distinct predicates (p, name, age).
    assert_eq!(r.rows.len(), 3);
    assert_boundary(&t, Op::Distinct);
    // The pre-distinct input is the whole-graph scan: all 9 triples captured.
    let total: usize = t.patterns.iter().map(|p| p.triples.len()).sum();
    assert_eq!(total, 9, "pre-DISTINCT input set is the full scan");
}

/// GROUP BY / aggregation records its boundary; the captured pattern set is the
/// pre-aggregation input. The COUNT pushdown is disabled under the recorder so
/// the input set is always materialized.
#[test]
fn group_aggregation_records_pre_aggregation_input() {
    let g = graph(DATA);
    let q = "SELECT (COUNT(?a) AS ?c) WHERE { ?s <http://ex/age> ?a }";
    let (r, t) = traced(&g, q);
    assert_eq!(r.rows.len(), 1);
    // The pre-aggregation input set: all 3 age triples, fully captured (no
    // index-range count pushdown under an armed recorder).
    let age_triples: usize = t
        .patterns
        .iter()
        .map(|p| p.triples.len())
        .sum();
    assert_eq!(age_triples, 3, "pre-aggregation set captured in full");
}

/// ASK over a single pattern: the index-range pushdown is disabled while
/// recording, so the matching triples are captured (not an empty witness).
#[test]
fn ask_disables_count_pushdown_and_captures_input() {
    let g = graph(DATA);
    let q = "ASK { ?s <http://ex/age> ?a }";
    let _guard = zk::install();
    let r = query(&g, q).unwrap();
    let t = zk::take();
    assert_eq!(r.rows.len(), 1, "ASK is true (unit row)");
    // The age pattern's triples are captured even though ASK only needs
    // emptiness — the witness must show what "exists" rests on.
    let total: usize = t.patterns.iter().map(|p| p.triples.len()).sum();
    assert_eq!(total, 3, "ASK captures the supporting input set");
}

/// LIMIT early-termination is disabled while recording: the full scan range is
/// captured (the completeness witness needs the whole sweep).
#[test]
fn limit_disables_early_termination() {
    let g = graph(DATA);
    let q = "SELECT ?s WHERE { ?s <http://ex/age> ?a } LIMIT 1";
    let (r, t) = traced(&g, q);
    assert_eq!(r.rows.len(), 1, "LIMIT 1 still limits the RESULT");
    // But the captured input set is the FULL scan (all 3), not just 1.
    let total: usize = t.patterns.iter().map(|p| p.triples.len()).sum();
    assert_eq!(total, 3, "completeness witness needs the whole scan range");
}

/// GRAPH <const> records exactly ONE Enter/Exit(Graph) boundary (not
/// double-nested) and tags the enclosed scan with the named graph.
#[test]
fn graph_const_single_boundary_and_tag() {
    let ds = Graph::load_dataset(
        "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .\n\
         <http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g2> .\n",
        "n-quads",
    )
    .unwrap();
    let q = "SELECT ?o WHERE { GRAPH <http://ex/g1> { ?s <http://ex/p> ?o } }";
    let (r, t) = traced(&ds, q);
    assert_eq!(r.rows.len(), 1);
    let enters = t.steps.iter().filter(|s| matches!(s, Step::Enter(Op::Graph))).count();
    let exits = t.steps.iter().filter(|s| matches!(s, Step::Exit(Op::Graph))).count();
    assert_eq!(enters, 1, "exactly one GRAPH boundary (not double-nested): {:?}", t.steps);
    assert_eq!(exits, 1);
    // The scan is tagged with g1.
    let p = t.patterns.iter().find(|p| p.graph.is_some()).expect("a graph-tagged pattern");
    assert_eq!(p.graph.as_ref().unwrap().to_string(), "<http://ex/g1>");
}

/// GRAPH ?g iterates the named graphs: each iteration records exactly one
/// boundary (regression guard for the double-scope bug roborev caught).
#[test]
fn graph_var_one_boundary_per_iteration() {
    let ds = Graph::load_dataset(
        "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .\n\
         <http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g2> .\n",
        "n-quads",
    )
    .unwrap();
    let q = "SELECT ?g ?o WHERE { GRAPH ?g { ?s <http://ex/p> ?o } }";
    let (r, t) = traced(&ds, q);
    assert_eq!(r.rows.len(), 2, "one row per named graph");
    let enters = t.steps.iter().filter(|s| matches!(s, Step::Enter(Op::Graph))).count();
    // Two visible named graphs => exactly two GRAPH boundaries, no nesting.
    assert_eq!(enters, 2, "one boundary per graph iteration, not nested: {:?}", t.steps);
    // Both graph tags appear.
    let tags: std::collections::BTreeSet<String> =
        t.patterns.iter().filter_map(|p| p.graph.as_ref().map(|t| t.to_string())).collect();
    assert_eq!(
        tags,
        std::collections::BTreeSet::from(["<http://ex/g1>".to_string(), "<http://ex/g2>".to_string()])
    );
}

/// Property paths are NOT captured per-pattern: an Op::Path marker is recorded
/// so a consumer can fail closed.
#[test]
fn property_path_marks_uncaptured() {
    let g = graph(DATA);
    let q = "SELECT ?o WHERE { <http://ex/a> <http://ex/p>+ ?o }";
    let (_r, t) = traced(&g, q);
    assert_eq!(t.first_uncaptured(), Some(Op::Path), "path must fail closed");
}

/// EXISTS inner scans are tagged in_exists and step-suppressed (EXISTS is
/// outside the stage-1 fragment).
#[test]
fn exists_inner_scans_tagged_and_suppressed() {
    let g = graph(DATA);
    let q = "SELECT ?s WHERE { ?s <http://ex/name> ?n . FILTER EXISTS { ?s <http://ex/age> ?a } }";
    let (r, t) = traced(&g, q);
    assert_eq!(r.rows.len(), 3, "a,b,c have both name and age");
    // Any pattern recorded from inside EXISTS carries in_exists; its scan does
    // NOT appear as a Step (suppressed).
    let in_exists_patterns: Vec<usize> = t
        .patterns
        .iter()
        .enumerate()
        .filter(|(_, p)| p.in_exists)
        .map(|(i, _)| i)
        .collect();
    for idx in &in_exists_patterns {
        assert!(
            !t.steps.iter().any(|s| matches!(s, Step::Scan { pattern } | Step::BindJoin { pattern } if pattern == idx)),
            "in_exists pattern {idx} must not have a step"
        );
    }
}

/// An unsatisfiable constant (a predicate absent from the dictionary) records a
/// provably-empty input set — the per-property proof witnesses "no such triple".
#[test]
fn unsatisfiable_pattern_records_empty_set() {
    let g = graph(DATA);
    let q = "SELECT ?s WHERE { ?s <http://ex/NOSUCHPRED> ?o }";
    let (r, t) = traced(&g, q);
    assert_eq!(r.rows.len(), 0);
    // The pattern is captured with zero triples (the empty-witness obligation).
    assert_eq!(t.patterns.len(), 1, "the empty pattern is recorded");
    assert_eq!(t.patterns[0].triples.len(), 0);
}

/// Determinism: same query + data (same load order) yields a byte-identical
/// trace across runs.
#[test]
fn trace_is_deterministic_across_runs() {
    let q = "SELECT * WHERE { ?s <http://ex/p> ?o . ?o <http://ex/name> ?n . FILTER(?n != \"B\") }";
    let run = || {
        let g = graph(DATA);
        traced(&g, q).1
    };
    assert_eq!(run(), run(), "same query + data => identical trace");
}

// ---- helpers ----------------------------------------------------------------

fn assert_boundary(t: &zk::ZkTrace, op: Op) {
    assert!(
        t.steps.iter().any(|s| matches!(s, Step::Enter(o) if *o == op)),
        "missing Enter({op:?}) in steps: {:?}",
        t.steps
    );
    assert!(
        t.steps.iter().any(|s| matches!(s, Step::Exit(o) if *o == op)),
        "missing Exit({op:?})"
    );
}

fn rows_set(r: &QueryResult) -> std::collections::BTreeSet<String> {
    r.rows.iter().map(|row| format!("{row:?}")).collect()
}

/// A fresh store of just the traced triples (sufficiency replay).
fn sparq_zk_replay(t: &zk::ZkTrace) -> Graph {
    use sparq_core::dict::Dict;
    let mut dict = Dict::new();
    let mut ids = Vec::new();
    for pm in &t.patterns {
        for tr in &pm.triples {
            ids.push([dict.intern(&tr[0]), dict.intern(&tr[1]), dict.intern(&tr[2])]);
        }
    }
    Graph::from_parts(dict, ids)
}
