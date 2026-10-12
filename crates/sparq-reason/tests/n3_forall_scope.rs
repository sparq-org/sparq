//! GH #6754: a formula-level `@forAll` binds only in the formula that declares it.
//!
//! The parser names such a universal by its declared IRI plus its de Bruijn distance to
//! the declaring formula (`?__uf.<n>.<iri>`), so the name is unforgeable (a `.` cannot
//! occur in a source quickvar) and canonical (alpha-equivalent copies of a formula are the
//! same term). In a rule, the premise's own universals are the rule's matching variables;
//! every other formula-level universal belongs to its quoted formula, matches only itself
//! and is never substituted. Document-level `@forAll` is unchanged.

use sparq_reason::n3::{parser, Term};
use sparq_reason::{reason_n3_pass_all, reason_n3_terms, RuleVars};

const EX: &str = "http://ex/";

fn iri(local: &str) -> Term {
    Term::Iri(format!("{EX}{local}"))
}

fn closure(src: &str) -> Vec<[Term; 3]> {
    reason_n3_terms(&format!("@prefix : <{EX}>.\n{src}"), None).expect("reasons").facts
}

fn has(facts: &[[Term; 3]], s: &str, p: &str, o: &str) -> bool {
    facts.contains(&[iri(s), iri(p), iri(o)])
}

/// Every variable name in `t`, at any depth.
fn vars(t: &Term, out: &mut Vec<String>) {
    match t {
        Term::Var(v) => out.push(v.clone()),
        Term::List(ms) => ms.iter().for_each(|m| vars(m, out)),
        Term::Triple(tr) => tr.iter().for_each(|m| vars(m, out)),
        Term::Formula(ts) => ts.iter().flatten().for_each(|m| vars(m, out)),
        _ => {}
    }
}

#[test]
fn rule_premise_and_conclusion_declarations_are_different_variables() {
    // The conclusion's `:x` is its own universal: the premise binds nothing in it, so no
    // ground fact follows (main derived `:c :q :b` through one shared variable).
    let facts = closure("{ @forAll :x. :x :p :b } => { @forAll :x. :x :q :b }.\n:c :p :b.\n");
    assert!(!has(&facts, "c", "q", "b"), "{facts:?}");
    // The premise's own universal still matches like a rule variable.
    let facts = closure("{ @forAll :x. :x :p :b } => { :hit :by :rule }.\n:c :p :b.\n");
    assert!(has(&facts, "hit", "by", "rule"), "{facts:?}");
}

#[test]
fn a_premise_universal_does_not_capture_a_quickvar_spelled_like_a_generated_name() {
    // The #6746 counterexample: no source quickvar can spell a generated binder name.
    let facts = closure("{ @forAll :x. :x :p :b } => { ?__uf0_x :q :b }.\n:c :p :b.\n");
    assert!(!has(&facts, "c", "q", "b"), "{facts:?}");
    let facts = closure("{ @forAll :x. :x :p :b } => { ?__up_x :q :b }.\n:c :p :b.\n");
    assert!(!has(&facts, "c", "q", "b"), "{facts:?}");
}

#[test]
fn a_quoted_sibling_copy_of_the_premise_keeps_its_own_binder() {
    // The conclusion quotes a formula identical to the premise. Its `:x` is bound in that
    // quoted formula, not by the premise, so the firing must not substitute into it.
    let src = "{ @forAll :x. :x :p :b } => { :log :says { @forAll :x. :x :p :b } }.\n:c :p :b.\n\
               :want :is { @forAll :x. :x :p :b }.\n";
    let facts = closure(src);
    let want = facts.iter().find(|f| f[0] == iri("want")).expect("source fact")[2].clone();
    let said: Vec<&Term> = facts.iter().filter(|f| f[0] == iri("log")).map(|f| &f[2]).collect();
    assert_eq!(said, vec![&want], "one firing, quoting the formula unchanged: {facts:?}");
}

#[test]
fn copies_of_one_closed_formula_are_equal() {
    let src = ":a :p { @forAll :x. :x :q :z }.\n:b :p { @forAll :x. :x :q :z }.\n\
               { :a :p ?f. :b :p ?g. ?f log:equalTo ?g } => { :same :is :yes }.\n\
               { :a :p ?f. :b :p ?g. ?f log:notEqualTo ?g } => { :differ :is :yes }.\n";
    let facts = closure(src);
    assert!(has(&facts, "same", "is", "yes"), "{facts:?}");
    assert!(!has(&facts, "differ", "is", "yes"), "{facts:?}");
    // Nested: an inner mention of the outer binder is the same term in both copies.
    let src = ":a :p { @forAll :x. :x :q { :x :r :z } }.\n:b :p { @forAll :x. :x :q { :x :r :z } }.\n\
               { :a :p ?f. :b :p ?g. ?f log:equalTo ?g } => { :same :is :yes }.\n";
    assert!(has(&closure(src), "same", "is", "yes"));
    // And a quoted copy written as a rule pattern matches the fact.
    let src = ":a :p { @forAll :x. :x :q :z }.\n{ :a :p { @forAll :x. :x :q :z } } => { :quoted :match :yes }.\n";
    assert!(has(&closure(src), "quoted", "match", "yes"));
}

#[test]
fn a_nested_declaration_shadows_the_outer_one() {
    // `{ :x :r :z }` mentions the OUTER `:x` in the first formula and its OWN `:x` in the
    // second, so the two formulae differ (main read both as one variable: equal).
    let src = ":a :p { @forAll :x. :x :q { :x :r :z } }.\n\
               :b :p { @forAll :x. :x :q { @forAll :x. :x :r :z } }.\n\
               { :a :p ?f. :b :p ?g. ?f log:notEqualTo ?g } => { :shadow :distinct :yes }.\n";
    assert!(has(&closure(src), "shadow", "distinct", "yes"));
    let p = parser::parse(&format!("@prefix : <{EX}>.\n{}", src.lines().take(2).collect::<String>())).unwrap();
    let mut a = Vec::new();
    vars(&p.facts[0][2], &mut a);
    let mut b = Vec::new();
    vars(&p.facts[1][2], &mut b);
    assert_eq!(a[0], b[0], "the outer binder at the outer formula's top level");
    assert_ne!(a[1], b[1], "outer binder vs the inner formula's own binder: {a:?} {b:?}");
}

#[test]
fn a_quoted_universal_in_a_premise_matches_only_itself() {
    // `{ @forAll :x. :x :q :z }` in a premise is a closed quoted formula: it is not the
    // ground `{ :k :q :z }`, and a rule variable cannot bind to its bound `:x`.
    let src = ":a :says { :k :q :z }.\n{ :a :says { @forAll :x. :x :q :z } } => { :matched :ground :yes }.\n";
    assert!(!has(&closure(src), "matched", "ground", "yes"));
    let src = ":a :says { @forAll :x. :x :q :z }.\n{ :a :says { ?s :q :z } } => { :w :saw { ?s :r :z } }.\n";
    let facts = closure(src);
    assert!(!facts.iter().any(|f| f[0] == iri("w")), "a bound universal does not escape: {facts:?}");
}

#[test]
fn a_formula_s_own_universals_quantify_its_rules_under_log_conclusion() {
    let src = ":k :is { @forAll :x. :c :p :b. { :x :p :b } => { :x :q :b } }.\n\
               { :k :is ?f. ?f log:conclusion ?g. ?g log:includes { :c :q :b } } => { :closure :fires :yes }.\n";
    assert!(has(&closure(src), "closure", "fires", "yes"));
}

#[test]
fn a_nested_closure_keeps_the_enclosing_formula_s_universals() {
    // The inner `log:conclusion` runs inside the outer one, over a formula that mentions the
    // OUTER formula's `:x`; its result must still mention that `:x`, not claim it.
    let src = ":k :is { @forAll :x. :a :says { :x :q :z }. { :a :says ?g. ?g log:conclusion ?h } => { :a :closed ?h } }.\n\
               { :k :is ?f. ?f log:conclusion ?c. ?c log:includes { @forAll :x. :a :closed { :x :q :z } } } => { :nested :lift :ok }.\n";
    assert!(has(&closure(src), "nested", "lift", "ok"));
}

#[test]
fn document_level_for_all_is_unchanged() {
    let facts = closure("@forAll :x. { :x :p :b } => { :x :q :b }.\n:c :p :b.\n");
    assert!(has(&facts, "c", "q", "b"), "{facts:?}");
    let p = parser::parse(&format!("@prefix : <{EX}>. @forAll :x. :a :p {{ :x :q :z }}. {{ :x :p :b }} => {{ :x :q :b }}.")).unwrap();
    let want = Term::Var(format!("__ua.{EX}x"));
    let mut v = Vec::new();
    vars(&p.facts[0][2], &mut v);
    assert_eq!(v, vec![format!("__ua.{EX}x")]);
    assert_eq!(p.rules[0].premise[0][0], want);
    assert_eq!(p.rules[0].conclusion[0][0], want);
}

#[test]
fn no_generated_binder_reaches_a_derived_fact() {
    let src = "{ @forAll :x. :x :p :b } => { :r :s { @forAll :y. :y :t :x } }.\n:c :p :b.\n";
    let facts = closure(src);
    for f in &facts {
        let mut v = Vec::new();
        f.iter().for_each(|t| vars(t, &mut v));
        assert!(v.iter().all(|n| !n.starts_with("__up.") && !n.starts_with("__uc.")), "{f:?}");
    }
}

#[test]
fn formula_level_universals_round_trip_through_pass_all() {
    let src = format!(
        "@prefix : <{EX}>.\n:a :p {{ @forAll :x. :x :q {{ @forAll :x. :x :r :z }} }}.\n\
         {{ @forAll :x. :x :p :b }} => {{ @forAll :x. :x :q :b. :log :says {{ @forAll :x. :x :p :b }} }}.\n:c :p :b.\n"
    );
    let doc = reason_n3_pass_all(&src, RuleVars::N3).expect("pass-all writes formula-level universals");
    assert!(!doc.contains("__u"), "no engine-internal name: {doc}");
    assert!(doc.contains("{ @forAll <http://ex/x> ."), "declared inside its formula: {doc}");
    let (a, b) = (parser::parse(&src).unwrap(), parser::parse(&doc).unwrap());
    assert_eq!(a.rules.len(), b.rules.len());
    assert_eq!(a.rules[0].premise, b.rules[0].premise, "{doc}");
    assert_eq!(a.rules[0].conclusion, b.rules[0].conclusion, "{doc}");
    assert_eq!(doc, reason_n3_pass_all(&doc, RuleVars::N3).expect("round two"));
}
