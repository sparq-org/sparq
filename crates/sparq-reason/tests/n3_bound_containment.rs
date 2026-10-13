//! GH #6936: a `log:includes` / `log:notIncludes` pattern bound through a variable is the
//! bound formula, exactly as if it were written inline.
//!
//! A formula scope matches it syntactically. A store scope runs it as a premise over the
//! closure; the automatic stratification cannot see what a variable pattern reads, so it
//! counts a store-scoped `log:notIncludes ?f` as a negation over every predicate (a
//! negation cycle, refused by default), and a store-scoped `log:includes ?f` as a
//! positive read of every predicate whose bound pattern may not itself negate over the
//! store (the run is refused when it does).

use sparq_reason::n3::{parser, Term};
use sparq_reason::{reason_n3_terms, reason_n3_terms_with_cycles, MaterializedN3Graph, NegationCycles};

const EX: &str = "http://ex/";
const DATA: &str = ":a :p :b. :cfg :hit { :a :p :b }. :cfg :miss { :c :p :b }.";

fn iri(local: &str) -> Term {
    Term::Iri(format!("{EX}{local}"))
}

fn doc(body: &str) -> String {
    format!("@prefix : <{EX}>. @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n{DATA}\n{body}")
}

/// The pattern each `:cfg` key holds.
fn pattern(key: &str) -> &'static str {
    match key {
        "hit" => "{ :a :p :b }",
        _ => "{ :c :p :b }",
    }
}

/// The rule for `op` / `key` over `scope`, with the pattern bound through `?f` or inline.
fn rule(op: &str, key: &str, scope: &str, bound: bool) -> String {
    let pat = if bound { "?f".to_string() } else { pattern(key).to_string() };
    format!("{{ :cfg :{key} ?f. {scope} log:{op} {pat} }} => {{ :r :{op} :{key} }}.")
}

/// The `:r` facts of a closure.
fn results(facts: &[[Term; 3]]) -> Vec<[Term; 3]> {
    let mut out: Vec<_> = facts.iter().filter(|t| t[0] == iri("r")).cloned().collect();
    out.sort_by_key(|t| format!("{t:?}"));
    out
}

fn text(src: &str, cycles: NegationCycles) -> Result<Vec<[Term; 3]>, String> {
    reason_n3_terms_with_cycles(src, None, None, cycles).map(|c| results(&c.facts))
}

fn incremental(src: &str) -> Result<Vec<[Term; 3]>, String> {
    let base = parser::parse(src).expect("parses").facts;
    MaterializedN3Graph::new(src, &base).map(|g| results(&g.closure()))
}

fn fired(op: &str, key: &str) -> [Term; 3] {
    [iri("r"), iri(op), iri(key)]
}

/// The truth table every engine must reproduce: `{ :a :p :b }` (and the store) includes
/// the `:hit` pattern and not the `:miss` one.
fn expected(op: &str, key: &str) -> bool {
    (op == "includes") == (key == "hit")
}

/// The issue's reproduction, for a formula scope: the bound pattern is matched.
#[test]
fn a_bound_pattern_is_matched_against_a_formula_scope() {
    for op in ["includes", "notIncludes"] {
        for key in ["hit", "miss"] {
            let bound = doc(&rule(op, key, "{ :a :p :b }", true));
            let inline = doc(&rule(op, key, "{ :a :p :b }", false));
            let want: Vec<_> = expected(op, key).then(|| fired(op, key)).into_iter().collect();
            assert_eq!(text(&bound, NegationCycles::Reject).expect("text"), want, "{bound}");
            assert_eq!(text(&inline, NegationCycles::Reject).expect("text"), want, "{inline}");
            assert_eq!(incremental(&bound).expect("incremental"), want, "{bound}");
            assert_eq!(incremental(&inline).expect("incremental"), want, "{inline}");
        }
    }
}

/// A store-scoped `log:includes` of a bound pattern reads the closure like the inline one.
#[test]
fn a_bound_pattern_is_matched_against_the_store() {
    for key in ["hit", "miss"] {
        let bound = doc(&rule("includes", key, "?scope", true));
        let inline = doc(&rule("includes", key, "?scope", false));
        let want: Vec<_> = expected("includes", key).then(|| fired("includes", key)).into_iter().collect();
        assert_eq!(text(&bound, NegationCycles::Reject).expect("text"), want, "{bound}");
        assert_eq!(text(&inline, NegationCycles::Reject).expect("text"), want, "{inline}");
        assert_eq!(incremental(&bound).expect("incremental"), want, "{bound}");
        assert_eq!(incremental(&inline).expect("incremental"), want, "{inline}");
    }
}

/// A store-scoped `log:notIncludes` of a bound pattern negates over every predicate the
/// pattern could mention, its own rule's conclusion included: the default refuses it as
/// a negation cycle; the opt-ins evaluate it as they do any such cycle.
#[test]
fn a_bound_store_scoped_negation_is_a_negation_cycle() {
    for key in ["hit", "miss"] {
        let bound = doc(&rule("notIncludes", key, "?scope", true));
        let err = text(&bound, NegationCycles::Reject).expect_err("refused");
        assert!(err.contains("stratification"), "{err}");
        let err = incremental(&bound).expect_err("refused");
        assert!(err.contains("stratification"), "{err}");
        // FailClosed derives nothing from the refused rule.
        assert_eq!(text(&bound, NegationCycles::FailClosed).expect("fail-closed"), none());
        // SinglePass matches the bound pattern, as the inline-written one.
        let want: Vec<_> = expected("notIncludes", key).then(|| fired("notIncludes", key)).into_iter().collect();
        assert_eq!(text(&bound, NegationCycles::SinglePass).expect("single pass"), want, "{bound}");
        let inline = doc(&rule("notIncludes", key, "?scope", false));
        assert_eq!(text(&inline, NegationCycles::Reject).expect("text"), want, "{inline}");
    }
}

/// The stratification hazard itself: the bound pattern names a predicate another rule
/// derives. Written inline, the negation waits for the derivation; bound through a
/// variable, the analysis cannot see the predicate, so it does not run the rule early.
#[test]
fn a_bound_negation_over_a_derived_predicate_is_not_evaluated_early() {
    let rules = "{ :a :p :b } => { :x :q :y }.
        { :cfg :late ?f. ?scope log:notIncludes ?f } => { :r :notIncludes :late }.";
    let src = format!("{} :cfg :late {{ :x :q :y }}.", doc(rules));
    let inline = src.replace("log:notIncludes ?f", "log:notIncludes { :x :q :y }");
    assert_eq!(text(&inline, NegationCycles::Reject).expect("stratified"), none());
    let err = text(&src, NegationCycles::Reject).expect_err("refused");
    assert!(err.contains("stratification"), "{err}");
    let closed = reason_n3_terms_with_cycles(&src, None, None, NegationCycles::FailClosed).expect("fail-closed");
    assert_eq!(results(&closed.facts), none());
    assert!(closed.facts.contains(&[iri("x"), iri("q"), iri("y")]));
}

/// A store-scoped `log:includes` whose bound pattern itself negates over the store would
/// run a negation the stratification never saw: the run is refused.
#[test]
fn a_bound_store_pattern_that_negates_is_refused() {
    for inner in [
        ":s log:notIncludes { :x :q :y }",
        "( :t { :x :q :y } ?l ) log:collectAllIn :s",
    ] {
        let src = format!(
            "{} :cfg :neg {{ {inner} }}.",
            doc("{ :cfg :neg ?f. ?scope log:includes ?f } => { :r :includes :neg }.")
        );
        let err = reason_n3_terms(&src, None).err().expect("refused");
        assert!(err.contains("bound through a variable"), "{src}: {err}");
        // A formula scope matches the same pattern syntactically: nothing to stratify.
        let local = src.replace("?scope log:includes ?f", "{ :a :p :b } log:includes ?f");
        assert_eq!(text(&local, NegationCycles::Reject).expect("formula scope"), none(), "{local}");
    }
}

/// A non-formula object is no pattern: the premise fails for both operators (it used to
/// match as the empty pattern, so `log:includes` held vacuously).
#[test]
fn a_non_formula_pattern_fails_the_premise() {
    for op in ["includes", "notIncludes"] {
        let src = format!(
            "{} :cfg :atom :b.",
            doc(&format!("{{ :cfg :atom ?f. {{ :a :p :b }} log:{op} ?f }} => {{ :r :{op} :atom }}."))
        );
        assert_eq!(text(&src, NegationCycles::Reject).expect("text"), none(), "{src}");
    }
}

/// The compiled evaluator has no variable pattern: it refuses the rule, so its callers
/// route the document to the batch engine instead of diverging from it.
#[cfg(feature = "compiled-rules")]
#[test]
fn the_compiled_evaluator_refuses_a_bound_pattern() {
    for op in ["includes", "notIncludes"] {
        let src = doc(&rule(op, "miss", "?scope", true));
        assert!(sparq_reason::n3::compiled::compile(&src).is_err(), "{src}");
    }
    // The same negation written inline over a plain pattern is in the compiled subset.
    let inline = format!("@prefix : <{EX}>. @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
        {{ :a :p ?o. ?scope log:notIncludes {{ :c :p ?o }} }} => {{ :r :notIncludes :miss }}.");
    assert!(sparq_reason::n3::compiled::compile(&inline).is_ok(), "{inline}");
}

fn none() -> Vec<[Term; 3]> {
    Vec::new()
}
