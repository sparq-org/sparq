//! sq-rd2: ASK / EXISTS only need to know whether >= 1 solution
//! exists. These tests pin (a) boolean CORRECTNESS for ASK and EXISTS / NOT
//! EXISTS, and (b) that the evaluator does not OVER-EVALUATE — proven with a
//! `max_rows` working-set budget set BELOW the full match count: full
//! materialisation would breach it (a hard error), whereas an early-exit that
//! stops at the first solution stays within it.
use super::*;
use crate::QueryBudget;

/// A graph with `n` subjects each `:a`-typed `:Thing`, plus a `:flag true`
/// pair, so a match set far larger than any small `max_rows` budget exists.
fn big(n: usize) -> Graph {
    let mut nt = String::new();
    for i in 0..n {
        nt.push_str(&format!("<http://ex/s{i}> <http://ex/p> <http://ex/Thing> .\n"));
    }
    Graph::load_str(&nt, "ntriples").unwrap()
}

/// A `max_rows`-only budget (no deadline): the working-set bound used as the
/// over-evaluation tripwire.
fn rows_budget(max: usize) -> QueryBudget {
    QueryBudget { max_rows: Some(max), ..QueryBudget::unlimited() }
}

#[test]
fn ask_true_false_correctness() {
    let g = Graph::load_str(
        "PREFIX : <http://ex/>\n:a :p :b . :b :p :c . :a :knows :z .",
        "turtle",
    )
    .unwrap();
    let ask = |q: &str| crate::ask(&g, &format!("PREFIX : <http://ex/> {q}")).unwrap();
    // Present / absent single pattern.
    assert!(ask("ASK { :a :p :b }"));
    assert!(!ask("ASK { :a :p :MISSING }"));
    // Variable pattern with >=1 / 0 matches.
    assert!(ask("ASK { ?s :p ?o }"));
    assert!(!ask("ASK { ?s :nosuch ?o }"));
    // Join: ?x :p ?y . ?y :p ?z  (a-p->b-p->c exists).
    assert!(ask("ASK { ?x :p ?y . ?y :p ?z }"));
    assert!(!ask("ASK { ?x :p ?y . ?y :knows ?z }")); // b/c have no :knows
    // FILTER.
    assert!(ask("ASK { ?s :p ?o FILTER(?o = :b) }"));
    assert!(!ask("ASK { ?s :p ?o FILTER(?o = :nope) }"));
}

#[test]
fn exists_in_filter_correctness() {
    let g = Graph::load_str(
        "PREFIX : <http://ex/>\n\
             :a :p :b . :b :p :c . :c :p :d .\n\
             :a :flagged true . :c :flagged true .",
        "turtle",
    )
    .unwrap();
    let n = |q: &str| crate::query(&g, &format!("PREFIX : <http://ex/> {q}")).unwrap().len();
    // EXISTS: subjects that have an outgoing :p AND are :flagged -> a, c.
    assert_eq!(n("SELECT ?s WHERE { ?s :flagged true FILTER EXISTS { ?s :p ?o } }"), 2);
    // NOT EXISTS: flagged subjects with NO outgoing :p -> none (a,c both have :p).
    assert_eq!(n("SELECT ?s WHERE { ?s :flagged true FILTER NOT EXISTS { ?s :p ?o } }"), 0);
    // Correlated EXISTS that is sometimes false: subjects with a :p whose object
    // is itself :flagged -> only :b's target :c is flagged... :a-p->:b (b not
    // flagged), :b-p->:c (c flagged), :c-p->:d (d not flagged) => 1 (the b row).
    assert_eq!(n("SELECT ?s WHERE { ?s :p ?o FILTER EXISTS { ?o :flagged true } }"), 1);
    // Uncorrelated EXISTS (constant) is true once -> keeps all 3 :p subjects.
    assert_eq!(n("SELECT ?s WHERE { ?s :p ?o FILTER EXISTS { :a :flagged true } }"), 3);
    // Uncorrelated EXISTS that is false -> drops everything.
    assert_eq!(n("SELECT ?s WHERE { ?s :p ?o FILTER EXISTS { :a :flagged false } }"), 0);
}

#[test]
fn ask_single_pattern_early_exit_does_not_over_evaluate() {
    // 10_000 matching triples; a working-set budget of 4 rows. A full
    // materialisation builds 10_000 rows and breaches the budget (hard error);
    // an early-exiting ASK answers from the index range size (or the first
    // solution) without ever materialising > 4 rows.
    let g = big(10_000);
    // SELECT proves the budget genuinely bites on full materialisation.
    assert!(
        crate::query_with_budget(&g, "SELECT * WHERE { ?s <http://ex/p> ?o }", &rows_budget(4)).is_err(),
        "control: full SELECT must breach the 4-row working-set budget"
    );
    // ASK must NOT breach it — it stops at the first solution / counts from the index.
    let got = crate::ask_with_budget(&g, "ASK { ?s <http://ex/p> ?o }", &rows_budget(4));
    assert_eq!(got, Ok(true), "ASK over a large single-pattern match set must early-exit, got {got:?}");
    // And a FALSE ASK over the same large store still returns cleanly (no match,
    // nothing materialised).
    assert_eq!(
        crate::ask_with_budget(&g, "ASK { ?s <http://ex/nosuch> ?o }", &rows_budget(4)),
        Ok(false)
    );
}

#[test]
fn exists_early_exit_does_not_over_evaluate() {
    // The outer query binds ONE row (:probe); its FILTER EXISTS scans a
    // 10_000-row inner match set. A full inner materialisation breaches a small
    // working-set budget; an early-exiting EXISTS stops at the first inner
    // solution. (The probe is uncorrelated so any inner solution satisfies it.)
    let mut nt = String::from("<http://ex/probe> <http://ex/kind> <http://ex/q> .\n");
    for i in 0..10_000 {
        nt.push_str(&format!("<http://ex/s{i}> <http://ex/p> <http://ex/Thing> .\n"));
    }
    let g = Graph::load_str(&nt, "ntriples").unwrap();
    let q = "SELECT ?x WHERE { ?x <http://ex/kind> <http://ex/q> \
                 FILTER EXISTS { ?y <http://ex/p> <http://ex/Thing> } }";
    // Control: the inner pattern as a standalone SELECT breaches the budget.
    assert!(
        crate::query_with_budget(&g, "SELECT * WHERE { ?y <http://ex/p> <http://ex/Thing> }", &rows_budget(8)).is_err(),
        "control: the inner 10k-row scan must breach an 8-row budget"
    );
    // The full query (1 outer row, EXISTS over 10k inner): must answer true
    // WITHOUT materialising the whole inner set, so it stays within the budget.
    let got = crate::query_with_budget(&g, q, &rows_budget(8));
    assert!(
        got.as_ref().map(|r| r.len()) == Ok(1),
        "EXISTS must early-exit on the inner scan, got {got:?}"
    );
}
