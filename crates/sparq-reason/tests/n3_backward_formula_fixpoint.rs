//! GH #6757: a backward rule that concludes a quoted formula holding a variable (or an
//! `@forAll` universal), copied out by a forward rule, must let the forward closure reach
//! its fixpoint. Each backward application standardizes the rule apart under fresh names,
//! so without a canonical naming every round derived a fact that differed from the last
//! only by a renaming, and the closure never saturated.

use std::sync::mpsc;
use std::time::Duration;

use sparq_reason::n3::{reason_n3_pass_all, reason_n3_terms, RuleVars, Term};

const EX: &str = "http://example.org/#";

fn iri(local: &str) -> Term {
    Term::Iri(format!("{EX}{local}"))
}

/// Run `f` on its own thread and fail the test if it does not finish in time, so a
/// diverging closure fails the test instead of hanging the suite.
fn terminates<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|_| panic!("{what} did not reach a fixpoint"))
}

/// The `:a :r ?o` objects of the closure of `doc`, after checking that every entry point
/// terminates on it. `pass_all` must write the closure, or — when it holds a
/// backward-chaining copy of an `@forAll` universal, which has no exact N3 spelling — refuse
/// it as such (`a_freshened_backward_rule_universal_is_refused` in `n3_pass_all.rs`).
fn r_objects(doc: &'static str) -> Vec<Term> {
    for vars in [RuleVars::N3, RuleVars::VarIris] {
        match terminates("reason_n3_pass_all", move || reason_n3_pass_all(doc, vars)) {
            Ok(out) => assert!(out.contains("<http://example.org/#r>"), "{out}"),
            Err(e) => assert!(doc.contains("@forAll") && e.contains("backward-chaining cop"), "{e}"),
        }
    }
    let closure = terminates("reason_n3_terms", move || reason_n3_terms(doc, None)).expect("terms");
    closure.facts.into_iter().filter(|t| t[0] == iri("a") && t[1] == iri("r")).map(|t| t[2].clone()).collect()
}

/// The single triple of a one-triple formula.
fn only_triple(t: &Term) -> &[Term; 3] {
    match t {
        Term::Formula(ts) if ts.len() == 1 => &ts[0],
        other => panic!("expected a one-triple formula, got {other:?}"),
    }
}

fn var_name(t: &Term) -> &str {
    match t {
        Term::Var(v) => v,
        other => panic!("expected a variable, got {other:?}"),
    }
}

#[test]
fn backward_formula_with_variable_reaches_fixpoint() {
    let objs = r_objects(
        "@prefix : <http://example.org/#>.\n\
         { :a :p { ?x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r ?f } .\n",
    );
    assert_eq!(objs.len(), 1, "one fact up to renaming: {objs:?}");
    let tr = only_triple(&objs[0]);
    assert!(matches!(tr[0], Term::Var(_)), "the formula keeps its variable: {tr:?}");
    assert_eq!((&tr[1], &tr[2]), (&iri("q"), &iri("z")));
}

#[test]
fn backward_formula_with_universal_reaches_fixpoint() {
    let objs = r_objects(
        "@prefix : <http://example.org/#>.\n\
         @forAll :x .\n\
         { :a :p { :x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r ?f } .\n",
    );
    assert_eq!(objs.len(), 1, "one fact up to renaming: {objs:?}");
    let tr = only_triple(&objs[0]);
    // Still a copy of the universal `:x`, not a plain variable or the IRI.
    assert!(var_name(&tr[0]).ends_with(&format!("__ua.{EX}x")), "{tr:?}");
    assert_eq!((&tr[1], &tr[2]), (&iri("q"), &iri("z")));
}

#[test]
fn backward_formula_nested_in_consequent_reaches_fixpoint() {
    let objs = r_objects(
        "@prefix : <http://example.org/#>.\n\
         @forAll :x .\n\
         { :a :p { :x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r { :s :t ?f } } .\n",
    );
    assert_eq!(objs.len(), 1, "one fact up to renaming: {objs:?}");
    let outer = only_triple(&objs[0]);
    assert_eq!((&outer[0], &outer[1]), (&iri("s"), &iri("t")));
    let inner = only_triple(&outer[2]);
    assert!(var_name(&inner[0]).ends_with(&format!("__ua.{EX}x")), "{inner:?}");
}

#[test]
fn backward_formula_to_another_subject_reaches_fixpoint() {
    let doc = "@prefix : <http://example.org/#>.\n\
               @forAll :x .\n\
               { :a :p { :x :q :z } } <= true .\n\
               { :a :p ?f } => { :b :r ?f } .\n";
    let closure = terminates("reason_n3_terms", move || reason_n3_terms(doc, None)).expect("terms");
    assert_eq!(closure.facts.iter().filter(|t| t[0] == iri("b")).count(), 1, "{:?}", closure.facts);
}

/// Canonical naming must not merge formulas that are genuinely different: different
/// constants, and one variable versus two.
#[test]
fn distinct_backward_formulas_all_survive() {
    let objs = r_objects(
        "@prefix : <http://example.org/#>.\n\
         { :a :p { ?x :q :z } } <= true .\n\
         { :a :p { ?x :q :w } } <= true .\n\
         { :a :p { ?x :q ?y } } <= true .\n\
         { :a :p { ?x :q ?x } } <= true .\n\
         { :a :p ?f } => { :a :r ?f } .\n",
    );
    assert_eq!(objs.len(), 4, "four genuinely different formulas: {objs:?}");
    let triples: Vec<&[Term; 3]> = objs.iter().map(only_triple).collect();
    assert!(triples.iter().any(|t| t[2] == iri("z")));
    assert!(triples.iter().any(|t| t[2] == iri("w")));
    let two_vars = triples.iter().filter(|t| matches!((&t[0], &t[2]), (Term::Var(s), Term::Var(o)) if s != o)).count();
    let one_var = triples.iter().filter(|t| matches!((&t[0], &t[2]), (Term::Var(s), Term::Var(o)) if s == o)).count();
    assert_eq!((two_vars, one_var), (1, 1), "{objs:?}");
}
