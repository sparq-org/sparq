//! The `cs-planner` ACCEPTANCE GATE (sparq-introspect TODO, "Gate when wired"):
//! characteristic-set estimates must beat the `PredStat` independence estimator on
//! **q-error** over a star-query workload. Both estimators are measured through the
//! REAL planner recurrence (`goo_seed` / `goo_pick` / `record_pattern_ndv` — the
//! exact arithmetic `eval_bgp_binary` runs), with and without a [`crate::cs::CsTable`]
//! installed, against TRUE cardinalities from full evaluation.
//!
//! Two workloads:
//! * synthetic correlated entity shapes (always runs — the CI gate), and
//! * the olympics benchmark dataset (1.78M triples; `#[ignore]`d — run with
//!   `cargo test -p sparq-engine --release --features cs-planner -- --ignored cs_gate --nocapture`
//!   when `bench/qlever-olympics/olympics.nt` is present).

use crate::cs::{CsSet, CsTable};
use crate::exec::{goo_pick, goo_seed, prepare_bgp, record_pattern_ndv, CsCtx};
use oxrdf::Variable;
use rustc_hash::FxHashMap;
use sparq_core::dict::Id;
use sparq_core::Graph;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use std::sync::Arc;

/// The star BGP `?s <p1> ?o0 . ?s <p2> ?o1 . …` over the given predicate ids.
fn star_patterns(graph: &Graph, preds: &[Id]) -> Vec<TriplePattern> {
    let s = Variable::new_unchecked("s");
    preds
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let oxrdf::Term::NamedNode(nn) = graph.dict.term(p) else { panic!("predicate is an IRI") };
            TriplePattern {
                subject: TermPattern::Variable(s.clone()),
                predicate: NamedNodePattern::NamedNode(nn),
                object: TermPattern::Variable(Variable::new_unchecked(format!("o{i}"))),
            }
        })
        .collect()
}

/// The planner's FINAL cardinality estimate for a BGP: the exact `cur_card`
/// recurrence of `eval_bgp_binary`, executed symbolically (no scans, no joins).
/// With a CS table installed this is the CS estimator; without, the PredStat one.
fn goo_estimate(graph: &Graph, patterns: &[TriplePattern]) -> f64 {
    let prepared = prepare_bgp(graph, patterns).unwrap();
    if prepared.iter().any(|p| p.unsatisfiable) {
        return 0.0;
    }
    let mut cs_ctx = CsCtx::new(&prepared);
    let seed = goo_seed(&prepared);
    let mut done = vec![false; prepared.len()];
    done[seed] = true;
    cs_ctx.note_done(seed);
    let mut cur_card = prepared[seed].est as f64;
    let mut var_ndv: FxHashMap<Variable, f64> = FxHashMap::default();
    record_pattern_ndv(graph, &prepared, seed, cur_card, &mut var_ndv, &cs_ctx);
    for _ in 1..prepared.len() {
        let (i, new_card, _) = goo_pick(graph, &prepared, &done, &var_ndv, cur_card, &cs_ctx);
        cur_card = new_card;
        done[i] = true;
        cs_ctx.note_done(i);
        record_pattern_ndv(graph, &prepared, i, cur_card, &mut var_ndv, &cs_ctx);
    }
    cur_card
}

/// True star cardinality, by full evaluation.
fn true_card(graph: &Graph, preds: &[Id]) -> usize {
    let where_clause: String = preds
        .iter()
        .enumerate()
        .map(|(i, &p)| format!("?s {} ?o{i} . ", graph.dict.term(p)))
        .collect();
    crate::count(graph, &format!("SELECT * WHERE {{ {where_clause}}}")).unwrap()
}

/// q-error with the standard floor: `max(est/true, true/est)`, both floored at 1.
fn q_error(est: f64, truth: usize) -> f64 {
    let est = est.max(1.0);
    let truth = (truth as f64).max(1.0);
    (est / truth).max(truth / est)
}

struct QStats {
    median: f64,
    gmean: f64,
    max: f64,
}

fn stats(mut qs: Vec<f64>) -> QStats {
    qs.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    let median = qs[qs.len() / 2];
    let gmean = (qs.iter().map(|q| q.ln()).sum::<f64>() / qs.len() as f64).exp();
    QStats { median, gmean, max: *qs.last().unwrap() }
}

fn cs_table_of(graph: &Graph) -> Arc<CsTable> {
    Arc::new(CsTable::new(sparq_introspect::characteristic_set_ids(graph).into_iter().map(|s| CsSet {
        predicates: s.predicates,
        subjects: s.subjects,
        predicate_triples: s.predicate_triples,
    })))
}

/// Builds the star workload from the graph's own characteristic sets: prefixes
/// (sizes 2..=4) of the top sets — correlated predicates a real query would star
/// over — plus cross-set predicate pairs (low/zero correlation, where the
/// independence assumption typically overestimates hardest).
fn star_workload(graph: &Graph, top_sets: usize) -> Vec<Vec<Id>> {
    let sets = sparq_introspect::characteristic_set_ids(graph);
    let mut workload: Vec<Vec<Id>> = Vec::new();
    let top: Vec<_> = sets.iter().filter(|s| s.predicates.len() >= 2).take(top_sets).collect();
    for s in &top {
        for k in 2..=s.predicates.len().min(4) {
            workload.push(s.predicates[..k].to_vec());
        }
        if s.predicates.len() > 4 {
            workload.push(s.predicates[s.predicates.len() - 2..].to_vec());
        }
    }
    for i in 0..top.len() {
        for j in (i + 1)..top.len() {
            let (a, b) = (top[i].predicates.last().unwrap(), top[j].predicates.last().unwrap());
            if a != b {
                let mut q = vec![*a, *b];
                q.sort_unstable();
                q.dedup();
                workload.push(q);
            }
        }
    }
    workload.sort_unstable();
    workload.dedup();
    workload
}

/// Runs both estimators over the workload; returns (predstat q-errors, cs q-errors).
fn run_gate(graph: &Graph, workload: &[Vec<Id>], verbose: bool) -> (Vec<f64>, Vec<f64>) {
    let table = cs_table_of(graph);
    let mut ps_q = Vec::new();
    let mut cs_q = Vec::new();
    for preds in workload {
        let patterns = star_patterns(graph, preds);
        let truth = true_card(graph, preds);
        let ps_est = goo_estimate(graph, &patterns);
        let cs_est = crate::cs::with_cs_table(&table, || goo_estimate(graph, &patterns));
        ps_q.push(q_error(ps_est, truth));
        cs_q.push(q_error(cs_est, truth));
        if verbose {
            eprintln!(
                "star |Q|={} true={truth:>9} predstat={ps_est:>12.1} (q={:>9.2}) cs={cs_est:>12.1} (q={:.2})",
                preds.len(),
                q_error(ps_est, truth),
                q_error(cs_est, truth),
            );
        }
    }
    (ps_q, cs_q)
}

fn assert_cs_wins(name: &str, ps_q: Vec<f64>, cs_q: Vec<f64>) {
    let n = ps_q.len();
    let (ps, cs) = (stats(ps_q), stats(cs_q));
    eprintln!(
        "[{name}] {n} star queries — q-error  PredStat: median {:.2} gmean {:.2} max {:.1}  |  CS: median {:.2} gmean {:.2} max {:.1}",
        ps.median, ps.gmean, ps.max, cs.median, cs.gmean, cs.max
    );
    assert!(
        cs.gmean < ps.gmean && cs.median <= ps.median && cs.max <= ps.max,
        "GATE FAILED on {name}: CS (median {:.2}, gmean {:.2}, max {:.1}) must beat PredStat (median {:.2}, gmean {:.2}, max {:.1})",
        cs.median,
        cs.gmean,
        cs.max,
        ps.median,
        ps.gmean,
        ps.max
    );
}

/// Synthetic correlated entity shapes (the always-on CI gate): three overlapping
/// predicate sets whose co-occurrence the independence model cannot see — type A
/// {p1,p2,p3}, type B {p1,p4}, type C {p2,p4,p5} — with non-unit multiplicities.
#[test]
fn cs_gate_synthetic_correlated_stars() {
    let mut nt = String::new();
    let mut t = |s: &str, p: u32, o: String| nt.push_str(&format!("<http://ex/{s}> <http://ex/p{p}> {o} .\n"));
    for i in 0..1000 {
        let s = format!("a{i}");
        t(&s, 1, format!("\"{i}\""));
        t(&s, 2, format!("<http://ex/v{}>", i % 50)); // mult 2 for p2 on type A
        t(&s, 2, format!("<http://ex/w{}>", i % 50));
        t(&s, 3, format!("\"{}\"", i % 7));
    }
    for i in 0..500 {
        let s = format!("b{i}");
        t(&s, 1, format!("\"{i}\""));
        t(&s, 4, format!("<http://ex/u{}>", i % 20));
    }
    for i in 0..200 {
        let s = format!("c{i}");
        t(&s, 2, format!("<http://ex/v{}>", i % 50));
        t(&s, 4, format!("<http://ex/u{}>", i % 20));
        t(&s, 5, format!("\"x{}\"", i % 3));
        t(&s, 5, format!("\"y{}\"", i % 3)); // mult 2 for p5 on type C
    }
    let graph = Graph::load_str(&nt, "ntriples").unwrap();
    // Resolve predicate ids by IRI.
    let pid = |n: u32| {
        graph
            .id_of(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(format!("http://ex/p{n}"))))
            .unwrap()
    };
    let workload: Vec<Vec<Id>> = vec![
        vec![pid(1), pid(2)],         // A only (B has p1 but not p2)
        vec![pid(1), pid(3)],         // A only
        vec![pid(2), pid(3)],         // A only
        vec![pid(1), pid(4)],         // B only
        vec![pid(2), pid(4)],         // C only
        vec![pid(3), pid(4)],         // EMPTY: no shape has both
        vec![pid(2), pid(5)],         // C only
        vec![pid(4), pid(5)],         // C only
        vec![pid(1), pid(5)],         // EMPTY
        vec![pid(1), pid(2), pid(3)], // A, with p2 mult 2
        vec![pid(2), pid(4), pid(5)], // C, with p5 mult 2
        vec![pid(1), pid(2), pid(4)], // EMPTY
    ]
    .into_iter()
    .map(|mut q| {
        q.sort_unstable();
        q
    })
    .collect();

    // Sanity: the CS estimator is EXACT on the builder's own data for pure stars.
    let table = cs_table_of(&graph);
    for q in &workload {
        let truth = true_card(&graph, q) as f64;
        let est = table.star_cardinality(q);
        assert!((est - truth).abs() < 1e-6, "CS star estimate must be exact: Q={q:?} est={est} true={truth}");
    }

    let (ps_q, cs_q) = run_gate(&graph, &workload, true);
    assert_cs_wins("synthetic", ps_q, cs_q);
}

/// An installed CS table may only change JOIN ORDER — query RESULTS must be
/// identical with and without it (including multi-pattern stars, mixed
/// star/chain shapes, OPTIONAL and FILTER around the BGP).
#[test]
fn cs_table_never_changes_results() {
    let ttl = "@prefix : <http://ex/> .
        :a :p :x ; :q 1 ; :r :b . :b :p :y ; :q 2 . :c :p :x ; :s :a .
        :d :q 3 ; :r :c . :e :p :z ; :q 4 ; :r :a ; :s :b .";
    let graph = Graph::load_str(ttl, "turtle").unwrap();
    let table = cs_table_of(&graph);
    for q in [
        "PREFIX : <http://ex/> SELECT * WHERE { ?s :p ?o . ?s :q ?n }",
        "PREFIX : <http://ex/> SELECT * WHERE { ?s :p ?o . ?s :q ?n . ?s :r ?t }",
        "PREFIX : <http://ex/> SELECT * WHERE { ?s :r ?m . ?m :p ?o . ?s :q ?n }",
        "PREFIX : <http://ex/> SELECT ?s WHERE { ?s :p ?o . ?s :q ?n . FILTER(?n > 1) OPTIONAL { ?s :s ?w } } ORDER BY ?s",
        "PREFIX : <http://ex/> SELECT * WHERE { ?s :p ?a . ?s :p ?b . ?s :q ?n }",
    ] {
        let plain = crate::query(&graph, q).unwrap();
        let with_cs = crate::cs::with_cs_table(&table, || crate::query(&graph, q)).unwrap();
        assert_eq!(plain.vars, with_cs.vars, "vars diverged for {q}");
        let norm = |r: &crate::QueryResult| {
            let mut rows: Vec<String> = r.rows.iter().map(|row| format!("{row:?}")).collect();
            rows.sort();
            rows
        };
        assert_eq!(norm(&plain), norm(&with_cs), "results diverged for {q}");
    }
}

/// The olympics gate (1.78M triples). Skips (passes with a note) when the fixture
/// is absent; `#[ignore]`d so the default suite stays fast — run explicitly:
/// `cargo test -p sparq-engine --release --features cs-planner -- --ignored cs_gate_olympics --nocapture`
#[test]
#[ignore = "needs bench/qlever-olympics/olympics.nt — run in --release with --nocapture"]
fn cs_gate_olympics() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/qlever-olympics/olympics.nt");
    if !path.exists() {
        eprintln!("skipping: olympics.nt not present (downloaded benchmark fixture)");
        return;
    }
    let text = std::fs::read_to_string(&path).expect("read olympics.nt");
    let graph = Graph::load_str(&text, "ntriples").expect("parse olympics.nt");
    eprintln!("olympics: {} triples", graph.len());
    let workload = star_workload(&graph, 8);
    eprintln!("workload: {} star queries", workload.len());
    let (ps_q, cs_q) = run_gate(&graph, &workload, true);
    assert_cs_wins("olympics", ps_q, cs_q);
}
