//! N3 formula-scope builtins — `log:includes` / `notIncludes` / `supports` / `conclusion` /
//! `conjunction` / `parsedAsN3`, and the resolver-gated `log:semantics` / `log:content`.
//!
//! 🤖 SPARQ agent — sq-qcnn test-quality slice.
//!
//! These exercise the formula-containment + scope-closure machinery (the deepest dark region of
//! the N3 engine). Each case is hand-derived from cwm/EYE log: builtin semantics: a forward rule
//! whose premise is a formula-scope check fires iff the scope relation holds. We assert the EXACT
//! entailed triple is present when the relation holds and ABSENT when it must not — and, for the
//! binding modes, that the bound value is correct.

use sparq_reason::n3::{reason_n3_terms, reason_n3_terms_with_resolver, Resolver, Term};

/// Term-level closure of a prefixed N3 body.
fn closure(body: &str) -> Vec<[Term; 3]> {
    let src = format!(
        "@prefix : <http://ex/> .\n\
         @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
         @prefix math: <http://www.w3.org/2000/10/swap/math#> .\n{}",
        body
    );
    reason_n3_terms(&src, None).expect("reasoning failed").facts
}

fn iri(s: &str) -> Term {
    Term::Iri(format!("http://ex/{}", s))
}

fn has(facts: &[[Term; 3]], s: &str, p: &str, o: &str) -> bool {
    facts.iter().any(|t| *t == [iri(s), iri(p), iri(o)])
}

#[test]
fn log_includes_ground_scope_holds() {
    // The scope formula CONTAINS the pattern triple ⊢ fire.
    let f = closure("{ {:a :p :b} log:includes {:a :p :b} } => { :r :ok :yes } .");
    assert!(
        has(&f, "r", "ok", "yes"),
        "log:includes on a contained triple fires; got {:?}",
        f
    );
}

#[test]
fn log_includes_binds_a_pattern_variable() {
    // log:includes may BIND a free variable of the object pattern (one binding per match).
    let f = closure("{ {:a :p :b} log:includes {:a :p ?o} } => { :r :found ?o } .");
    assert!(
        has(&f, "r", "found", "b"),
        "log:includes binds ?o = :b; got {:?}",
        f
    );
}

#[test]
fn log_includes_absent_triple_does_not_fire() {
    // The scope does NOT contain the pattern ⊢ no firing.
    let f = closure("{ {:a :p :b} log:includes {:a :p :c} } => { :r :ok :yes } .");
    assert!(
        !has(&f, "r", "ok", "yes"),
        "log:includes on an absent triple must not fire; got {:?}",
        f
    );
}

#[test]
fn empty_formula_includes_nothing() {
    // The empty formula `{}` (= literal true) includes nothing — notIncludes everything.
    let f = closure("{ {} log:notIncludes {:a :p :b} } => { :r :empty :ok } .");
    assert!(
        has(&f, "r", "empty", "ok"),
        "{{}} notIncludes any triple; got {:?}",
        f
    );
    let f2 = closure("{ {} log:includes {:a :p :b} } => { :r :bad :yes } .");
    assert!(
        !has(&f2, "r", "bad", "yes"),
        "{{}} includes NOTHING; got {:?}",
        f2
    );
}

#[test]
fn log_not_includes_holds_iff_no_match() {
    // notIncludes holds when the scope lacks the pattern, fails when it has it.
    let f = closure("{ {:a :p :b} log:notIncludes {:a :p :c} } => { :r :ni :ok } .");
    assert!(
        has(&f, "r", "ni", "ok"),
        "notIncludes on an absent triple holds; got {:?}",
        f
    );
    let f2 = closure("{ {:a :p :b} log:notIncludes {:a :p :b} } => { :r :bad :yes } .");
    assert!(
        !has(&f2, "r", "bad", "yes"),
        "notIncludes on a present triple fails; got {:?}",
        f2
    );
}

#[test]
fn log_supports_closes_the_scope_under_its_own_rules() {
    // log:supports first runs the scope formula's OWN `=>` rules, THEN checks containment.
    // Scope: {:a :p :b} plus a rule {?x :p ?y}=>{?x :q ?y}. The closure entails (:a :q :b),
    // so the scope SUPPORTS {:a :q :b} even though it isn't asserted verbatim.
    let f = closure(
        "{ { :a :p :b . { ?x :p ?y } => { ?x :q ?y } } log:supports { :a :q :b } } \
         => { :r :supported :yes } .",
    );
    assert!(
        has(&f, "r", "supported", "yes"),
        "log:supports closes the scope under its rules then checks containment; got {:?}",
        f
    );
}

#[test]
fn log_conclusion_returns_the_scope_closure_as_a_formula() {
    // log:conclusion binds ?g to a formula = the scope's forward closure. We then use
    // log:includes to ASSERT the derived triple is in that conclusion formula.
    let f = closure(
        "{ { :a :p :b . { ?x :p ?y } => { ?x :q ?y } } log:conclusion ?g . \
           ?g log:includes { :a :q :b } } => { :r :concluded :yes } .",
    );
    assert!(
        has(&f, "r", "concluded", "yes"),
        "log:conclusion's formula includes the derived (:a :q :b); got {:?}",
        f
    );
}

#[test]
fn log_conjunction_merges_a_list_of_formulae() {
    // log:conjunction of ( {:a :p :b} {:c :p :d} ) is a single formula containing both;
    // log:includes confirms membership.
    let f = closure(
        "{ ( {:a :p :b} {:c :p :d} ) log:conjunction ?g . ?g log:includes {:c :p :d} } \
         => { :r :conj :yes } .",
    );
    assert!(
        has(&f, "r", "conj", "yes"),
        "conjunction merges both formulae; got {:?}",
        f
    );
}

#[test]
fn log_parsed_as_n3_parses_a_source_string_to_a_formula() {
    // log:parsedAsN3 parses an N3 source LITERAL into a formula; log:includes inspects it.
    let f = closure(
        "{ \"<http://ex/a> <http://ex/p> <http://ex/b> .\" log:parsedAsN3 ?g . \
           ?g log:includes {:a :p :b} } => { :r :parsed :yes } .",
    );
    assert!(
        has(&f, "r", "parsed", "yes"),
        "parsedAsN3 yields a formula with (:a :p :b); got {:?}",
        f
    );
}

#[test]
fn log_semantics_uses_the_caller_resolver() {
    // log:semantics is OFF unless the caller supplies a Resolver. With one that maps a doc IRI
    // to N3 text, the parsed formula is inspectable via log:includes.
    let src = "@prefix : <http://ex/> .\n\
               @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
               { <http://doc/d1> log:semantics ?g . ?g log:includes {:a :p :b} } \
               => { :r :sem :yes } .";
    let resolver = |iri: &str| {
        if iri == "http://doc/d1" {
            Some("<http://ex/a> <http://ex/p> <http://ex/b> .".to_string())
        } else {
            None
        }
    };
    let f = reason_n3_terms_with_resolver(src, None, Some(&resolver as &Resolver))
        .expect("resolver reasoning")
        .facts;
    assert!(
        has(&f, "r", "sem", "yes"),
        "log:semantics via resolver yields the doc formula; got {:?}",
        f
    );
}

#[test]
fn log_semantics_is_off_without_a_resolver() {
    // Without a Resolver the document-access builtins yield nothing — the premise fails closed,
    // preserving the "no I/O of its own" policy.
    let f = closure(
        "{ <http://doc/d1> log:semantics ?g . ?g log:includes {:a :p :b} } => { :r :sem :yes } .",
    );
    assert!(
        !has(&f, "r", "sem", "yes"),
        "log:semantics must do nothing without a resolver; got {:?}",
        f
    );
}

#[test]
fn log_content_returns_raw_document_text() {
    // log:content returns the document's SOURCE TEXT as a string literal (no parsing).
    let src = "@prefix : <http://ex/> .\n\
               @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
               @prefix string: <http://www.w3.org/2000/10/swap/string#> .\n\
               { <http://doc/d1> log:content ?c . ?c string:contains \"hello\" } \
               => { :r :content :yes } .";
    let resolver = |iri: &str| (iri == "http://doc/d1").then(|| "hello world".to_string());
    let f = reason_n3_terms_with_resolver(src, None, Some(&resolver as &Resolver))
        .expect("resolver reasoning")
        .facts;
    assert!(
        has(&f, "r", "content", "yes"),
        "log:content returns raw text containing 'hello'; got {:?}",
        f
    );
}
