//! GH #6936: with a FORMULA as the scope, a `log:includes` / `log:notIncludes` /
//! `log:supports` pattern bound through a variable is matched as the formula it is bound
//! to, fully substituted, when that formula is GROUND under the binding (blank nodes are
//! its existentials), exactly as the same formula written in the rule. Every other bound
//! value (a non-ground formula included), and every bound pattern with the STORE as the
//! scope, still matches as the empty pattern, as before (GH #6943): the stratification
//! does not read a pattern given through a variable.

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

/// The issue's reproduction for every operator with a formula scope: the ground bound
/// pattern is matched as the inline one is, on the text engine and the incremental graph
/// (which routes these rules to the batch engine).
#[test]
fn a_ground_bound_pattern_is_matched_as_written_inline() {
    for op in ["includes", "notIncludes", "supports"] {
        for key in ["hit", "miss"] {
            for bound in [true, false] {
                let src = doc(&rule(op, key, "{ :a :p :b }", bound));
                assert_eq!(text(&src, NegationCycles::Reject).expect("text"), want(op, key), "{src}");
                assert_eq!(incremental(&src).expect("incremental"), want(op, key), "{src}");
            }
        }
    }
}

/// With the store as the scope a bound pattern keeps the empty-pattern match, under every
/// negation-cycle mode: `log:includes` / `log:supports` hold and `log:notIncludes` fails,
/// whatever the pattern (GH #6943). The inline pattern is matched.
#[test]
fn a_bound_store_scoped_pattern_matches_as_the_empty_pattern() {
    for op in ["includes", "notIncludes", "supports"] {
        for key in ["hit", "miss"] {
            let src = doc(&rule(op, key, "?scope", true));
            let empty = if op == "notIncludes" { none() } else { fired(op, key) };
            for cycles in [NegationCycles::Reject, NegationCycles::FailClosed, NegationCycles::SinglePass] {
                assert_eq!(text(&src, cycles).expect("text"), empty, "{src}");
            }
            assert_eq!(incremental(&src).expect("incremental"), empty, "{src}");
            let inline = doc(&rule(op, key, "?scope", false));
            assert_eq!(text(&inline, NegationCycles::Reject).expect("inline"), want(op, key), "{inline}");
        }
    }
}

/// Why the store scope keeps the empty pattern: the stratification does not read a bound
/// pattern, so the rule below runs in the first stratum, before the later one derives the
/// pattern's fact. Matching the pattern there would miss a conclusion the empty-pattern
/// match derives, and the final store supports.
#[test]
fn a_bound_store_scoped_pattern_derived_in_a_later_stratum_still_holds() {
    let src = format!(
        "{} :cfg :late {{ :z :w :v }}.",
        doc(
            "{ :a :p :b } => { :m :n :o }.
             { ?s log:notIncludes { :m :n :o2 } } => { :z :w :v }.
             { :cfg :late ?f. ?scope log:includes ?f } => { :r :includes :late }."
        )
    );
    assert_eq!(text(&src, NegationCycles::Reject).expect("text"), fired("includes", "late"), "{src}");
}

/// A pattern whose nested formulas are bound through further variables is matched fully
/// substituted, not with the nested values compared as written: `?f = { :a :p ?o }`,
/// `?o = { :b :q ?x }`, `?x = :c` is the pattern `{ :a :p { :b :q :c } }`.
#[test]
fn a_bound_pattern_is_matched_fully_substituted() {
    let src = format!(
        "{} :cfg :f {{ :a :p ?o }}. :cfg :o {{ :b :q ?x }}. :cfg :x :c. :cfg :y :d.",
        doc(
            "{ :cfg :f ?f. :cfg :o ?o. :cfg :x ?x. { :a :p { :b :q :c } } log:includes ?f } => { :r :includes :nested }.
             { :cfg :f ?f. :cfg :o ?o. :cfg :x ?x. { :a :p { :b :q :c } } log:notIncludes ?f } => { :r :notIncludes :nested }.
             { :cfg :f ?f. :cfg :o ?o. :cfg :y ?x. { :a :p { :b :q :c } } log:notIncludes ?f } => { :r :notIncludes :other }."
        )
    );
    let mut want = fired("includes", "nested");
    want.extend(fired("notIncludes", "other"));
    want.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(text(&src, NegationCycles::Reject).expect("text"), want, "{src}");
    assert_eq!(incremental(&src).expect("incremental"), want, "{src}");
}

/// A bound pattern whose binding refers back to itself fails the run.
#[test]
fn a_cyclic_bound_pattern_fails_the_run() {
    let src = format!(
        "{} :cfg :cyc {{ :a :p ?f }}.",
        doc("{ :cfg :cyc ?f. { :a :p :b } log:includes ?f } => { :r :includes :cycle }.")
    );
    let err = text(&src, NegationCycles::Reject).expect_err("refused");
    assert!(err.contains("cyclic binding"), "{err}");
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
