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
// different node: on both maintenance paths the two must never merge, and the graph shows
// its own blanks in the reserved `N3_GRAPH_BLANK` space so the output never conflates them.

use sparq_reason::N3_GRAPH_BLANK;

fn blank(l: &str) -> Term {
    Term::Blank(l.into())
}

/// A rules-document blank as the graph shows it.
fn doc_blank(l: &str) -> Term {
    Term::Blank(format!("{N3_GRAPH_BLANK}{l}"))
}

/// Every fact `closure()` returns is `contains`ed, and no two returned facts are equal.
fn assert_closure_consistent(g: &MaterializedN3Graph) {
    let closure = g.closure();
    assert_eq!(set(closure.clone()).len(), closure.len(), "duplicate facts: {closure:?}");
    for t in &closure {
        assert!(g.contains(t), "closure() returned a fact contains() denies: {t:?}");
    }
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
        [doc_blank("_b1"), iri("p"), iri("o1")],
        [doc_blank("_b2"), iri("p"), iri("o2")],
        [doc_blank("x"), iri("p"), iri("o3")],
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
    assert_closure_consistent(&g);
    for t in &base {
        assert!(g.contains(t), "asserted base blank triple lost: {t:?}");
        assert!(g.contains(&[t[0].clone(), iri("seen"), t[2].clone()]));
    }
    // The document's `_:x` and the caller's `x` are two terms in the output.
    assert!(g.contains(&[doc_blank("x"), iri("p"), iri("o3")]));
    assert!(!g.contains(&[blank("x"), iri("p"), iri("o3")]));
    assert!(!g.contains(&[doc_blank("x"), iri("r"), iri("o3")]));
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
    assert_closure_consistent(&g);
    let base = base_blank_facts();
    assert_eq!(g.insert(&base), base.len());
    assert_eq!(g.mode(), mode);
    assert_eq!(set(g.closure()), expected_with(&base));
    assert_closure_consistent(&g);
    for t in &base {
        assert!(g.contains(t), "inserted base blank triple lost: {t:?}");
    }
    // Re-inserting is a no-op: the caller's label maps to the same node every time.
    assert_eq!(g.insert(&base), 0);
    assert_eq!(g.delete(&base[..1]), 1);
    assert!(!g.contains(&base[0]));
    assert_eq!(set(g.closure()), expected_with(&base[1..]));
    assert_closure_consistent(&g);
    // The caller's `_b1` is not the document's: the document's `[]` fact stays.
    assert_eq!(g.delete(&[[blank("_b1"), iri("p"), iri("o1")]]), 0);
    assert_eq!(set(g.closure()), expected_with(&base[1..]));
    assert_eq!(g.delete(&base[1..]), 2);
    assert_eq!(set(g.closure()), expected_with(&[]));
    assert_closure_consistent(&g);
    // A label in the reserved space names the graph's own node, as `closure()` shows it.
    assert!(g.contains(&doc_facts()[0]));
}

#[test]
fn fallback_base_blank_insert_delete_round_trip() {
    assert_base_blank_mutations(true);
}

#[test]
fn counting_base_blank_insert_delete_round_trip() {
    assert_base_blank_mutations(false);
}

#[test]
fn reserved_label_naming_no_node_is_refused() {
    // `outward` never shows a caller's own node in the reserved space, so a base label
    // that would name one there is an error, and is ignored by insert/contains.
    let bad = [blank(&format!("{N3_GRAPH_BLANK}__bs0_x")), iri("r"), iri("o1")];
    assert!(MaterializedN3Graph::new(&blank_rules(false), std::slice::from_ref(&bad)).is_err());
    let mut g = MaterializedN3Graph::new(&blank_rules(false), &[]).expect("parse");
    assert_eq!(g.insert(std::slice::from_ref(&bad)), 0);
    assert!(!g.contains(&bad));
}

/// A document parsed at run time (`log:parsedAsN3`) has blanks of its own: a text that
/// spells a caller's label, or the graph's internal namespace for it, never aliases the
/// caller's node.
#[test]
fn runtime_parsed_blanks_do_not_alias_caller_blanks() {
    for text in ["_:x", "_:__bs0_x"] {
        let rules = format!(
            "@prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             {{ \"{text} <http://ex/r> <http://ex/o9> .\" log:parsedAsN3 ?f . \
                ?f log:includes {{ ?s <http://ex/r> <http://ex/o9> }} }} \
             => {{ ?s <http://ex/fromtext> <http://ex/o9> }} .\n\
             {{ ?s <http://ex/fromtext> ?o . ?s <http://ex/r> ?o }} => {{ ?s <http://ex/both> ?o }} ."
        );
        let mine = [blank("x"), iri("r"), iri("o9")];
        let g = MaterializedN3Graph::new(&rules, std::slice::from_ref(&mine)).expect("parse");
        assert_eq!(g.mode(), N3Mode::Fallback, "{:?}", g.fallback_reason());
        let closure = g.closure();
        assert_closure_consistent(&g);
        let from_text: Vec<_> = closure.iter().filter(|t| t[1] == iri("fromtext")).collect();
        assert_eq!(from_text.len(), 1, "{text}: {closure:?}");
        let Term::Blank(l) = &from_text[0][0] else { panic!("{text}: {closure:?}") };
        assert!(l.starts_with(N3_GRAPH_BLANK), "{text}: runtime blank shown as {l:?}");
        assert!(!g.contains(&[blank("x"), iri("fromtext"), iri("o9")]), "{text}: {closure:?}");
        assert!(!closure.iter().any(|t| t[1] == iri("both")), "{text}: {closure:?}");
        assert!(g.contains(&mine));
    }
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
        assert!(g.why(&[blank("_b1"), iri("both"), iri("o1")]).is_none());
        // The document's `_:x` and the caller's `x`: two facts, two identity keys.
        let mine = g.why(&base[2]).expect("caller x explains");
        let theirs = g.why(&doc_facts()[2]).expect("document x explains");
        assert_ne!(mine.nodes()[0].key, theirs.nodes()[0].key, "fallback={fallback}");
        assert_ne!(mine.nodes()[0].key[0], theirs.nodes()[0].key[0]);
        assert_eq!(theirs.nodes()[0].rule, "asserted");
    }
}
