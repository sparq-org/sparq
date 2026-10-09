//! GH #6757: a backward rule that concludes a quoted formula holding a variable (or an
//! `@forAll` universal), copied out by a forward rule, must let the forward closure reach
//! its fixpoint. Each backward application standardizes the rule apart under fresh names,
//! so every round derived a fact that differed from the last only by that renaming, and
//! the closure never saturated. The closure now dedupes derived facts up to
//! alpha-equivalence of the variables inside their formulas; no variable is ever renamed.

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

/// Alpha-equivalent deduplication must not merge formulas that are genuinely different: different
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

/// The closure of `doc` as facts, after checking `reason_n3_terms` terminates on it.
fn closure_facts(doc: &'static str) -> Vec<[Term; 3]> {
    terminates("reason_n3_terms", move || reason_n3_terms(doc, None)).expect("terms").facts
}

/// Independent copies that share a base name (`?x` of two different backward rules) stay
/// independent variables: `?u` cannot be both of them, so `:bad :is true` is not derived.
#[test]
fn independent_backward_copies_are_not_identified() {
    let facts = closure_facts(
        "@prefix : <http://ex/>.\n\
         @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
         { :a :p { ?x :q :z } } <= true.\n\
         { :b :p { ?x :q :w } } <= true.\n\
         { ?f :pair ?g } <= { :a :p ?f. :b :p ?g }.\n\
         { ?f :pair ?g. ?f log:includes { ?u :q :z }. ?g log:includes { ?u :q :w } } => { :bad :is true }.\n",
    );
    assert!(!facts.iter().any(|t| t[0] == Term::Iri("http://ex/bad".into())), "{facts:?}");
}

/// A source variable is never renamed, whatever its spelling: the formula a backward rule
/// projects from an asserted fact is that fact's formula.
#[test]
fn source_variable_spelled_like_a_copy_keeps_its_identity() {
    let facts = closure_facts(
        "@prefix : <http://ex/>.\n\
         @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
         :a :seed { ?__bwa7_x :q :z }.\n\
         { :a :p ?f } <= { :a :seed ?f }.\n\
         { :a :seed ?s. :a :p ?f. ?s log:equalTo ?f } => { :result :same true }.\n",
    );
    assert!(
        facts.iter().any(|t| t[0] == Term::Iri("http://ex/result".into()) && t[1] == Term::Iri("http://ex/same".into())),
        "{facts:?}"
    );
}

/// Alpha-equivalence is joint per fact: a fact whose two formulas share their variable and
/// a fact whose formulas have independent variables are each alpha-equivalent formula by
/// formula, but they are different facts and both survive (once each).
#[test]
fn shared_and_independent_formula_variables_are_not_merged() {
    let facts = closure_facts(
        "@prefix : <http://example.org/#>.\n\
         { { ?x :q :z } :pair { ?x :q :w } } <= true .\n\
         { { ?x :q :z } :pair { ?y :q :w } } <= true .\n\
         { ?f :pair ?g } => { ?f :r ?g } .\n",
    );
    let pairs: Vec<&[Term; 3]> = facts.iter().filter(|t| t[1] == iri("r")).collect();
    assert_eq!(pairs.len(), 2, "shared and independent variables are different facts: {pairs:?}");
    let shares = |t: &[Term; 3]| var_name(&only_triple(&t[0])[0]) == var_name(&only_triple(&t[2])[0]);
    assert_eq!(pairs.iter().filter(|t| shares(t)).count(), 1, "{pairs:?}");
}

/// Two ASSERTED alpha-equivalent facts both stay: only derived facts dedupe up to alpha.
#[test]
fn asserted_alpha_equivalent_facts_both_stay() {
    let facts = closure_facts(
        "@prefix : <http://example.org/#>.\n\
         :a :r { ?x :q :z } .\n\
         :a :r { ?y :q :z } .\n\
         { :a :p { ?x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r ?f } .\n",
    );
    let objs: Vec<&Term> = facts.iter().filter(|t| t[0] == iri("a") && t[1] == iri("r")).map(|t| &t[2]).collect();
    assert_eq!(objs.len(), 2, "{objs:?}");
}

/// Every closure entry point terminates on the GH #6757 repros (each one runs the same
/// fixpoint loop; `every_closure_loop_inserts_derived_facts_up_to_alpha` guards that).
/// Interning entry points may refuse a non-ground formula in the closure; they must not hang.
#[test]
fn every_entry_point_terminates() {
    const DOCS: [&str; 2] = [
        "@prefix : <http://example.org/#>.\n\
         { :a :p { ?x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r ?f } .\n",
        "@prefix : <http://example.org/#>.\n\
         @forAll :x .\n\
         { :a :p { :x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r { :s :t ?f } } .\n",
    ];
    for doc in DOCS {
        let _ = terminates("reason_n3", move || sparq_reason::n3::reason_n3(&mut sparq_core::dict::Dict::new(), doc));
        let _ = terminates("reason_n3_proof_run", move || {
            sparq_reason::n3::reason_n3_proof_run(&mut sparq_core::dict::Dict::new(), doc).map(|r| r.closure.len())
        });
        let _ = terminates("reason_n3_stratified", move || {
            sparq_reason::n3::reason_n3_stratified(&mut sparq_core::dict::Dict::new(), &[doc, doc])
                .map(|r| r.strata_facts)
        });
        let _ = terminates("reason_n3_query_terms", move || {
            sparq_reason::n3::reason_n3_query_terms(
                doc,
                "@prefix : <http://example.org/#>. { :a :r ?f } => { :a :answer ?f } .",
            )
        });
        let g = terminates("MaterializedN3Graph::new (fallback)", move || {
            sparq_reason::MaterializedN3Graph::new(doc, &[]).map(|g| g.mode())
        });
        if let Ok(mode) = g {
            assert_eq!(mode, sparq_reason::N3Mode::Fallback, "backward rules run in fallback mode");
        }
    }
}
