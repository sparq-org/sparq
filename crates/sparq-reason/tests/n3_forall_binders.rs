//! `@forAll` binder identity in the N3 parser: a universal is the variable of the ONE
//! declaration that binds it. A formula-level declaration binds only inside its formula,
//! so two declarations of one IRI in different formulae are different variables; a
//! document-level declaration binds everywhere after it (a rule's premise and conclusion
//! share it).

use sparq_reason::n3::{parser, Term};
use sparq_reason::reason_n3_terms;

const PRE: &str = "@prefix : <http://ex/>. @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n";

fn iri(l: &str) -> Term {
    Term::Iri(format!("http://ex/{l}"))
}

fn derives(body: &str, f: [Term; 3]) -> bool {
    reason_n3_terms(&format!("{PRE}{body}"), None).expect("reasons").facts.contains(&f)
}

/// The variable names the universals of `:x` got, in text order.
fn universals(body: &str) -> Vec<String> {
    fn walk(t: &Term, out: &mut Vec<String>) {
        match t {
            Term::Var(v) if v.starts_with("__u") => out.push(v.clone()),
            Term::List(ms) => ms.iter().for_each(|m| walk(m, out)),
            Term::Triple(tr) => tr.iter().for_each(|m| walk(m, out)),
            Term::Formula(ts) => ts.iter().flatten().for_each(|m| walk(m, out)),
            _ => {}
        }
    }
    let p = parser::parse(&format!("{PRE}{body}")).expect("parses");
    let mut out = Vec::new();
    for t in p.facts.iter().flatten() {
        walk(t, &mut out);
    }
    for r in &p.rules {
        r.premise.iter().chain(&r.conclusion).flatten().for_each(|t| walk(t, &mut out));
    }
    out
}

/// A document-level universal is shared by a rule's premise and conclusion: the rule fires.
#[test]
fn a_document_level_universal_spans_premise_and_conclusion() {
    let names = universals("@forAll :x. { :x :p :b } => { :x :q :b }.");
    assert_eq!(names[0], names[1]);
    assert!(derives("@forAll :x. { :x :p :b } => { :x :q :b }.\n:c :p :b.\n", [iri("c"), iri("q"), iri("b")]));
}

/// Declared separately in the premise and in the conclusion, `:x` is two variables: the
/// conclusion's is not bound by the premise, so no ground `:c :q :b` follows. (Before
/// binder identity both declarations read as one variable and the rule derived it.)
#[test]
fn per_side_declarations_are_different_variables() {
    let body = "{ @forAll :x. :x :p :b } => { @forAll :x. :x :q :b }.\n:c :p :b.\n";
    let names = universals(body);
    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1]);
    assert!(!derives(body, [iri("c"), iri("q"), iri("b")]));
}

/// Sibling formulae and a nested formula that re-declares `:x` each have their own binder;
/// re-declaring in the SAME formula keeps the one variable.
#[test]
fn siblings_and_shadowing_keep_distinct_binders() {
    let s = universals(":a :p { :s :q { @forAll :x. :x :q :o }. :s :r { @forAll :x. :x :r :o } }.");
    assert_ne!(s[0], s[1], "siblings");
    let n = universals(":a :p { @forAll :x. :x :q { @forAll :x. :x :r :o }. :x :s :o }.");
    assert_eq!(n.len(), 3);
    assert_ne!(n[0], n[1], "the inner declaration shadows");
    assert_eq!(n[0], n[2], "the outer binder resumes after the inner formula");
    let same = universals(":a :p { @forAll :x. :x :q :o. @forAll :x. :x :r :o }.");
    assert_eq!(same[0], same[1], "one scope, one quantifier");
}

/// Formulae compare as terms (`log:equalTo`, matching), so two formulae with their OWN
/// formula-level binders are different terms, while a document-level universal makes them
/// equal. (Before binder identity the first pair compared equal.)
#[test]
fn formulae_with_their_own_binders_are_different_terms() {
    let rule = "{ :a :p ?f. :b :p ?g. ?f log:equalTo ?g } => { :same :is :yes }.\n";
    let own = format!(":a :p {{ @forAll :x. :x :q :z }}.\n:b :p {{ @forAll :x. :x :q :z }}.\n{rule}");
    assert!(!derives(&own, [iri("same"), iri("is"), iri("yes")]));
    let shared = format!("@forAll :x.\n:a :p {{ :x :q :z }}.\n:b :p {{ :x :q :z }}.\n{rule}");
    assert!(derives(&shared, [iri("same"), iri("is"), iri("yes")]));
}
