//! GH #6936: a `log:includes` / `log:notIncludes` / `log:supports` pattern bound through a
//! variable is matched as the formula it is bound to when that formula is GROUND under the
//! binding (blank nodes are its existentials), exactly as the same formula written in the
//! rule. Any other bound value, a non-ground formula included, still matches as the empty
//! pattern, as before (GH #6943).
//!
//! With the store as the scope, the automatic stratification cannot see what a variable
//! pattern reads: a `log:notIncludes ?f` counts as a negation over every predicate (a
//! negation cycle, refused by default), and a ground bound `log:includes` pattern that
//! itself negates over the store is refused at run time.

use sparq_reason::n3::{parser, Term};
use sparq_reason::{reason_n3_terms_with_cycles, MaterializedN3Graph, NegationCycles};

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

fn fired(op: &str, key: &str) -> Vec<[Term; 3]> {
    vec![[iri("r"), iri(op), iri(key)]]
}

fn none() -> Vec<[Term; 3]> {
    Vec::new()
}

/// The truth table: `{ :a :p :b }` (and the store) include the `:hit` pattern and not the
/// `:miss` one; `log:supports` agrees with `log:includes` (no rules in the scope).
fn want(op: &str, key: &str) -> Vec<[Term; 3]> {
    if (op == "notIncludes") == (key == "hit") {
        none()
    } else {
        fired(op, key)
    }
}

/// The issue's reproduction for every operator, with a formula and a store scope: the
/// ground bound pattern is matched, as the inline one is, on the text engine and the
/// incremental graph (which routes these rules to the batch engine).
#[test]
fn a_ground_bound_pattern_is_matched_as_written_inline() {
    for op in ["includes", "notIncludes", "supports"] {
        for scope in ["{ :a :p :b }", "?scope"] {
            // A store-scoped notIncludes of a bound pattern is a negation cycle (below).
            if op == "notIncludes" && scope == "?scope" {
                continue;
            }
            for key in ["hit", "miss"] {
                for bound in [true, false] {
                    let src = doc(&rule(op, key, scope, bound));
                    assert_eq!(text(&src, NegationCycles::Reject).expect("text"), want(op, key), "{src}");
                    assert_eq!(incremental(&src).expect("incremental"), want(op, key), "{src}");
                }
            }
        }
    }
}

/// A store-scoped `log:notIncludes` of a bound pattern negates over every predicate the
/// pattern could mention, its own rule's conclusion included: the default refuses it as a
/// negation cycle, FailClosed drops the rule, SinglePass matches the bound pattern.
#[test]
fn a_bound_store_scoped_negation_is_stratified_conservatively() {
    for key in ["hit", "miss"] {
        let bound = doc(&rule("notIncludes", key, "?scope", true));
        let err = text(&bound, NegationCycles::Reject).expect_err("refused");
        assert!(err.contains("stratification"), "{err}");
        let err = incremental(&bound).expect_err("refused");
        assert!(err.contains("stratification"), "{err}");
        assert_eq!(text(&bound, NegationCycles::FailClosed).expect("fail-closed"), none());
        assert_eq!(text(&bound, NegationCycles::SinglePass).expect("single pass"), want("notIncludes", key));
        let inline = doc(&rule("notIncludes", key, "?scope", false));
        assert_eq!(text(&inline, NegationCycles::Reject).expect("inline"), want("notIncludes", key));
    }
}

/// A store-scoped `log:includes` whose ground bound pattern itself negates over the store
/// would run a negation the stratification never saw: the run is refused.
#[test]
fn a_bound_store_pattern_that_negates_is_refused() {
    let src = format!(
        "{} :cfg :neg {{ :s log:notIncludes {{ :x :q :y }} }}.",
        doc("{ :cfg :neg ?f. ?scope log:includes ?f } => { :r :includes :neg }.")
    );
    let err = text(&src, NegationCycles::Reject).expect_err("refused");
    assert!(err.contains("bound through a variable holds a store-scoped"), "{err}");
}

/// Each containment tests its pattern's blank nodes afresh: a witness found by one test is
/// not carried into the next (on main the first witness leaked into the binding).
#[test]
fn existential_witnesses_do_not_leak_between_containments() {
    for pat in ["?f", "{ _:x :p :b }"] {
        let src = format!(
            "{} :cfg :blank {{ _:x :p :b }}.",
            doc(&format!(
                "{{ :cfg :blank ?f. {{ :a :p :b }} log:includes {pat}. {{ :c :p :b }} log:includes {pat} }} => {{ :r :includes :both }}.
                 {{ :cfg :blank ?f. {{ :a :p :b }} log:includes {pat}. {{ :c :p :b }} log:notIncludes {pat} }} => {{ :r :notIncludes :second }}."
            ))
        );
        assert_eq!(text(&src, NegationCycles::Reject).expect("text"), fired("includes", "both"), "{src}");
    }
}

/// A bound formula that is not ground keeps main's behaviour: it matches as the empty
/// pattern, so `log:includes` holds and `log:notIncludes` fails whatever the scope holds.
/// (Main derives exactly `:r :includes :open` here; matching such patterns is GH #6943.)
#[test]
fn a_non_ground_bound_pattern_behaves_as_on_main() {
    let src = format!(
        "{} :cfg :open {{ ?x :q :b }}.",
        doc(
            "{ :cfg :open ?f. { :a :p :b } log:includes ?f } => { :r :includes :open }.
             { :cfg :open ?f. { :a :p :b } log:notIncludes ?f } => { :r :notIncludes :open }."
        )
    );
    assert_eq!(text(&src, NegationCycles::Reject).expect("text"), fired("includes", "open"), "{src}");
    assert_eq!(incremental(&src).expect("incremental"), fired("includes", "open"), "{src}");
}
