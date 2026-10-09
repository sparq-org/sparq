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

// ---- GH #6775: base blank nodes never alias the rules document's blank nodes ----------
//
// The parser labels the document's anonymous `[]` nodes `_b1`, `_b2`, … and keeps a
// written `_:x` as `x`. A caller's base `Term::Blank("_b1")` / `Term::Blank("x")` is a
// different node: on both maintenance paths the two must never merge.

fn blank(l: &str) -> Term {
    Term::Blank(l.into())
}

/// Rules with document blanks `_b1`, `_b2` (from `[]`) and `x` (from `_:x`) in facts.
/// `{ ?s :p ?o . ?s :r ?o } => { ?s :both ?o }` fires only if a document blank and a base
/// blank were the same node. With `fallback`, an unsupported builtin forces the batch
/// engine.
fn blank_rules(fallback: bool) -> String {
    let force = if fallback {
        "{ ?s <http://ex/z> ?v . (1 2) math:sum ?n } => { ?s <http://ex/zz> ?n } .\n"
    } else {
        ""
    };
    format!(
        "@prefix math: <http://www.w3.org/2000/10/swap/math#> .\n\
         [] <http://ex/p> <http://ex/o1> .\n\
         [] <http://ex/p> <http://ex/o2> .\n\
         _:x <http://ex/p> <http://ex/o3> .\n\
         {{ ?s <http://ex/p> ?o . ?s <http://ex/r> ?o }} => {{ ?s <http://ex/both> ?o }} .\n\
         {{ ?s <http://ex/r> ?o }} => {{ ?s <http://ex/seen> ?o }} .\n\
         {force}"
    )
}

fn doc_facts() -> Vec<[Term; 3]> {
    vec![
        [blank("_b1"), iri("p"), iri("o1")],
        [blank("_b2"), iri("p"), iri("o2")],
        [blank("x"), iri("p"), iri("o3")],
    ]
}

fn base_blank_facts() -> Vec<[Term; 3]> {
    vec![
        [blank("_b1"), iri("r"), iri("o1")],
        [blank("_b2"), iri("r"), iri("o2")],
        [blank("x"), iri("r"), iri("o3")],
    ]
}

/// The closure over the document facts plus `base`: each base fact and its `:seen` copy,
/// with the caller's labels, and no `:both` (which would need a merge).
fn expected_with(base: &[[Term; 3]]) -> FxHashSet<[Term; 3]> {
    let mut e = set(doc_facts());
    for [s, _, o] in base {
        e.insert([s.clone(), iri("r"), o.clone()]);
        e.insert([s.clone(), iri("seen"), o.clone()]);
    }
    e
}

fn assert_base_blanks_stay_apart(fallback: bool) {
    let mode = if fallback { N3Mode::Fallback } else { N3Mode::Counting };
    let base = base_blank_facts();
    let g = MaterializedN3Graph::new(&blank_rules(fallback), &base).expect("parse");
    assert_eq!(g.mode(), mode, "{:?}", g.fallback_reason());
    let closure = set(g.closure());
    assert!(
        !closure.iter().any(|t| t[1] == iri("both")),
        "a base blank merged with a document blank: {closure:?}"
    );
    assert_eq!(closure, expected_with(&base));
    assert_eq!(g.len(), closure.len());
    for t in &base {
        assert!(g.contains(t), "asserted base blank triple lost: {t:?}");
        assert!(g.contains(&[t[0].clone(), iri("seen"), t[2].clone()]));
    }
}

#[test]
fn fallback_base_blanks_do_not_alias_document_blanks() {
    assert_base_blanks_stay_apart(true);
}

#[test]
fn counting_base_blanks_do_not_alias_document_blanks() {
    assert_base_blanks_stay_apart(false);
}

fn assert_base_blank_mutations(fallback: bool) {
    let mode = if fallback { N3Mode::Fallback } else { N3Mode::Counting };
    let mut g = MaterializedN3Graph::new(&blank_rules(fallback), &[]).expect("parse");
    assert_eq!(g.mode(), mode, "{:?}", g.fallback_reason());
    assert_eq!(set(g.closure()), expected_with(&[]));
    let base = base_blank_facts();
    assert_eq!(g.insert(&base), base.len());
    assert_eq!(g.mode(), mode);
    assert_eq!(set(g.closure()), expected_with(&base));
    for t in &base {
        assert!(g.contains(t), "inserted base blank triple lost: {t:?}");
    }
    // Re-inserting is a no-op: the caller's label maps to the same node every time.
    assert_eq!(g.insert(&base), 0);
    assert_eq!(g.delete(&base[..1]), 1);
    assert!(!g.contains(&base[0]));
    assert_eq!(set(g.closure()), expected_with(&base[1..]));
    // The document's own `_b1` fact is not the caller's: deleting by that label leaves it.
    assert_eq!(g.delete(&doc_facts()[..1]), 0);
    assert_eq!(set(g.closure()), expected_with(&base[1..]));
    assert_eq!(g.delete(&base[1..]), 2);
    assert_eq!(set(g.closure()), expected_with(&[]));
}

#[test]
fn fallback_base_blank_insert_delete_round_trip() {
    assert_base_blank_mutations(true);
}

#[test]
fn counting_base_blank_insert_delete_round_trip() {
    assert_base_blank_mutations(false);
}

#[cfg(feature = "explain")]
#[test]
fn why_shows_the_callers_blank_labels() {
    for fallback in [false, true] {
        let base = base_blank_facts();
        let g = MaterializedN3Graph::new(&blank_rules(fallback), &base).expect("parse");
        let seen = [blank("_b1"), iri("seen"), iri("o1")];
        let tree = g.why(&seen).expect("derived fact explains");
        let root = &tree.nodes()[tree.root() as usize];
        assert_eq!(root.conclusion[0], "_:_b1", "fallback={fallback}");
        assert_eq!(root.premises.len(), 1);
        let leaf = &tree.nodes()[root.premises[0] as usize];
        assert_eq!(leaf.rule, "asserted");
        assert_eq!(leaf.conclusion[0], "_:_b1", "fallback={fallback}");
        let asserted = g.why(&base[0]).expect("asserted fact explains");
        assert_eq!(asserted.nodes()[0].conclusion[0], "_:_b1");
        // The document's own `[]` fact is not the caller's `_b1`.
        assert!(g.why(&[blank("_b1"), iri("both"), iri("o1")]).is_none());
    }
}
