//! GH #6718: formula-scoped variables are values in batch conclusions, but the
//! counting emitter's recursive groundness check drops them.

use rustc_hash::FxHashSet;
use sparq_reason::n3::{reason_n3_terms_with_limits, ClosureLimits, NegationCycles, Term};
use sparq_reason::{reason_n3_terms, MaterializedN3Graph, N3Mode};

const RULES: &str = "@prefix : <http://ex/>. { :a :p ?f } => { :b :r ?f }.";
const FACT: &str = ":a :p { :c :q ?y }.";

fn batch(src: &str) -> FxHashSet<[Term; 3]> {
    reason_n3_terms(src, None)
        .unwrap()
        .facts
        .into_iter()
        .collect()
}

fn fact(src: &str) -> [Term; 3] {
    let facts = batch(&format!("@prefix : <http://ex/>. {src}"));
    assert_eq!(facts.len(), 1);
    facts.into_iter().next().unwrap()
}

fn assert_matches(g: &MaterializedN3Graph, src: &str, mode: N3Mode) {
    assert_eq!(
        g.closure().into_iter().collect::<FxHashSet<_>>(),
        batch(src)
    );
    assert_eq!(g.mode(), mode);
    assert_eq!(g.stratification_warning(), None);
    if mode == N3Mode::Fallback {
        let reason = g.fallback_reason().expect("fallback must explain itself");
        assert!(
            reason.contains("formula") && reason.contains("variable"),
            "{reason}"
        );
    } else {
        assert_eq!(g.fallback_reason(), None);
    }
}

#[test]
fn formula_variables_fall_back_at_construction_and_through_edits() {
    // Also cover formulas in subjects and nested in lists, triple terms and formulas.
    for (rules, data) in [
        (RULES, FACT),
        (
            "@prefix : <http://ex/>. { ?f :p :a } => { ?f :r :b }.",
            "{ :c :q ?y } :p :a .",
        ),
        (RULES, ":a :p ({ :c :q ?y })."),
        (RULES, ":a :p <<( :c :q { :d :q ?y } )>>."),
        (RULES, ":a :p { :c :q { :d :q ?y } }."),
    ] {
        let src = format!("{rules} {data}");
        assert_eq!(batch(&src).len(), 2, "fixture must derive a fact: {data}");
        let g = MaterializedN3Graph::new(&src, &[]).unwrap();
        assert_matches(&g, &src, N3Mode::Fallback);

        let asserted = [fact(data)];
        let g = MaterializedN3Graph::new(rules, &asserted).unwrap();
        assert_matches(&g, &src, N3Mode::Fallback);

        let mut g = MaterializedN3Graph::new(rules, &[]).unwrap();
        assert_matches(&g, rules, N3Mode::Counting);
        assert_eq!(g.insert(&asserted), 1);
        assert_matches(&g, &src, N3Mode::Fallback);
        assert_eq!(g.insert(&asserted), 0);
        assert_eq!(g.delete(&asserted), 1);
        assert_matches(&g, rules, N3Mode::Counting);
    }
}

#[test]
fn ground_values_keep_counting() {
    for data in [":a :p :c .", ":a :p { :c :q :d }.", ":a :p ({ :c :q :d })."] {
        let src = format!("{RULES} {data}");
        let g = MaterializedN3Graph::new(&src, &[]).unwrap();
        assert_matches(&g, &src, N3Mode::Counting);
        let mut g = MaterializedN3Graph::new(RULES, &[]).unwrap();
        let asserted = [fact(data)];
        assert_eq!(g.insert(&asserted), 1);
        assert_matches(&g, &src, N3Mode::Counting);
        assert_eq!(g.delete(&asserted), 1);
        assert_matches(&g, RULES, N3Mode::Counting);
    }
}

#[test]
fn insert_still_ignores_variables_outside_formulas() {
    let mut g = MaterializedN3Graph::new(RULES, &[]).unwrap();
    for value in [
        Term::Var("x".into()),
        Term::List(vec![Term::Var("x".into()), fact(FACT)[2].clone()]),
        Term::Triple(Box::new([
            Term::Var("x".into()),
            fact(FACT)[1].clone(),
            fact(FACT)[2].clone(),
        ])),
    ] {
        let mut asserted = fact(FACT);
        asserted[2] = value;
        assert_eq!(g.insert(&[asserted]), 0);
        assert_matches(&g, RULES, N3Mode::Counting);
    }
}

#[test]
fn formula_fallback_preserves_limits_and_recovers_after_delete() {
    let src = format!("{RULES} {FACT}");
    let mut limits = ClosureLimits::default();
    limits.max_facts = 1;
    let error = reason_n3_terms_with_limits(&src, None, None, NegationCycles::Reject, limits)
        .err()
        .expect("batch refused");
    assert!(error.contains("forward closure limit exceeded"), "{error}");
    let error = MaterializedN3Graph::with_limits(&src, &[], limits)
        .err()
        .expect("refused");
    assert!(error.contains("forward closure limit exceeded"), "{error}");

    let mut g = MaterializedN3Graph::with_limits(RULES, &[], limits).unwrap();
    let asserted = [fact(FACT)];
    assert_eq!(g.insert(&asserted), 1);
    assert_eq!(g.mode(), N3Mode::Fallback);
    assert!(g
        .stratification_warning()
        .unwrap()
        .contains("forward closure limit exceeded"));
    assert_eq!(
        g.closure(),
        asserted,
        "refusal must fail closed to the base"
    );
    assert_eq!(g.delete(&asserted), 1);
    assert_matches(&g, RULES, N3Mode::Counting);
}

/// Every fact of a fallback closure over formula values with variables is one that
/// `contains` reports (and, with `explain`, one `why` explains), whether the value was
/// asserted or a rule built it.
#[test]
fn fallback_closure_facts_are_readable() {
    for (rules, data) in [
        (RULES, FACT),
        (RULES, ":a :p ({ :c :q ?y })."),
        // A rule builds the formula value; another rule carries it on.
        (
            "@prefix : <http://ex/>. { :a :p :c } => { :b :r { :c :q ?y } }. \
             { :b :r ?f } => { :d :s ?f }.",
            ":a :p :c .",
        ),
    ] {
        let src = format!("{rules} {data}");
        let g = MaterializedN3Graph::new(&src, &[]).unwrap();
        assert_eq!(g.mode(), N3Mode::Fallback, "{src}");
        let closure = g.closure();
        assert_eq!(closure.iter().cloned().collect::<FxHashSet<_>>(), batch(&src));
        assert!(closure.len() >= 2, "fixture must derive a fact: {src}");
        for t in &closure {
            assert!(g.contains(t), "{t:?}");
            #[cfg(feature = "explain")]
            assert!(g.why(t).is_some(), "{t:?}");
        }
    }
}
