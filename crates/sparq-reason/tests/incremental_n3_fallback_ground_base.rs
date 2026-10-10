//! GH #6755: when [`MaterializedN3Graph`] falls back to the batch engine, the base
//! triples are ground data. The rules document's `@forAll` / `@forSome` directives
//! quantify the rules only; they must not turn an asserted base IRI into a variable.
//! The fallback closure must equal what the counting path keeps for the same base.

use rustc_hash::FxHashSet;
use sparq_reason::n3::Term;
use sparq_reason::{MaterializedN3Graph, N3Mode};

fn iri(local: &str) -> Term {
    Term::Iri(format!("http://ex/{local}"))
}
fn int(v: &str) -> Term {
    Term::Lit(
        v.into(),
        "http://www.w3.org/2001/XMLSchema#integer".into(),
        None,
    )
}
fn set(ts: Vec<[Term; 3]>) -> FxHashSet<[Term; 3]> {
    ts.into_iter().collect()
}

/// The issue's rules with the given quantifier directive over `<http://ex/a>`. The
/// unsupported `math:sum` builtin forces the rules-level fallback.
fn rules_quantifying_a(directive: &str) -> String {
    format!(
        "@prefix math: <http://www.w3.org/2000/10/swap/math#> .\n\
         {directive} <http://ex/a> .\n\
         {{ ?s <http://ex/p> ?v . (1 2) math:sum ?o }} => {{ ?s <http://ex/q> ?o }} ."
    )
}

fn assert_base_stays_ground(directive: &str) {
    let rules = rules_quantifying_a(directive);
    let asserted = [iri("a"), iri("p"), iri("v")];
    let g = MaterializedN3Graph::new(&rules, std::slice::from_ref(&asserted)).expect("parse");
    assert_eq!(
        g.mode(),
        N3Mode::Fallback,
        "{directive}: {:?}",
        g.fallback_reason()
    );
    assert!(
        g.contains(&asserted),
        "{directive}: asserted base triple lost: {:?}",
        g.closure()
    );
    // The from-scratch closure over the base: the asserted fact plus the one derivation.
    let expected = set(vec![asserted.clone(), [iri("a"), iri("q"), int("3")]]);
    assert_eq!(set(g.closure()), expected, "{directive}");
    assert_eq!(g.len(), expected.len(), "{directive}");
}

#[test]
fn for_all_in_rules_does_not_quantify_fallback_base() {
    assert_base_stays_ground("@forAll");
}

#[test]
fn for_some_in_rules_does_not_quantify_fallback_base() {
    assert_base_stays_ground("@forSome");
}

#[test]
fn fallback_base_survives_mutations() {
    let rules = rules_quantifying_a("@forAll");
    let asserted = [iri("a"), iri("p"), iri("v")];
    let mut g = MaterializedN3Graph::new(&rules, &[]).expect("parse");
    g.insert(std::slice::from_ref(&asserted));
    assert_eq!(g.mode(), N3Mode::Fallback);
    let other = [iri("b"), iri("p"), iri("w")];
    g.insert(std::slice::from_ref(&other));
    let expected = set(vec![
        asserted.clone(),
        other.clone(),
        [iri("a"), iri("q"), int("3")],
        [iri("b"), iri("q"), int("3")],
    ]);
    assert_eq!(set(g.closure()), expected);
    g.delete(std::slice::from_ref(&other));
    assert_eq!(
        set(g.closure()),
        set(vec![asserted, [iri("a"), iri("q"), int("3")]])
    );
}

#[test]
fn counting_and_fallback_keep_the_same_base() {
    // Same directive, no unsupported builtin: the counting path. Its closure keeps the
    // asserted triple, and so must the fallback above.
    let rules = "@forAll <http://ex/a> .\n{ ?s <http://ex/p> ?v } => { ?s <http://ex/q> ?v } .";
    let asserted = [iri("a"), iri("p"), iri("v")];
    let g = MaterializedN3Graph::new(rules, std::slice::from_ref(&asserted)).expect("parse");
    assert_eq!(g.mode(), N3Mode::Counting, "{:?}", g.fallback_reason());
    assert_eq!(
        set(g.closure()),
        set(vec![asserted, [iri("a"), iri("q"), iri("v")]])
    );
}

#[test]
fn quantified_iri_still_acts_as_a_variable_inside_the_rules() {
    // `<http://ex/x>` is universally quantified in the rules document: inside the rule it
    // matches any subject, while a base triple that spells `<http://ex/x>` stays ground.
    let rules = "@prefix math: <http://www.w3.org/2000/10/swap/math#> .\n\
                 @forAll <http://ex/x> .\n\
                 { <http://ex/x> <http://ex/p> ?v . (1 2) math:sum ?o } => { <http://ex/x> <http://ex/q> ?o } .";
    let b = [iri("b"), iri("p"), iri("v")];
    let x = [iri("x"), iri("p"), iri("w")];
    let g = MaterializedN3Graph::new(rules, &[b.clone(), x.clone()]).expect("parse");
    assert_eq!(g.mode(), N3Mode::Fallback, "{:?}", g.fallback_reason());
    let expected = set(vec![
        b,
        x,
        [iri("b"), iri("q"), int("3")],
        [iri("x"), iri("q"), int("3")],
    ]);
    assert_eq!(set(g.closure()), expected);
}

#[test]
fn fallback_base_blank_node_keeps_its_label() {
    // A base blank node reaches the engine as a term: no serialization round trip
    // relabels it, so the fallback closure holds the exact base term.
    let rules = rules_quantifying_a("@forAll");
    let b = Term::Blank("b1".into());
    let asserted = [b.clone(), iri("p"), iri("v")];
    let g = MaterializedN3Graph::new(&rules, std::slice::from_ref(&asserted)).expect("parse");
    assert_eq!(g.mode(), N3Mode::Fallback);
    assert_eq!(
        set(g.closure()),
        set(vec![asserted, [b, iri("q"), int("3")]])
    );
}
