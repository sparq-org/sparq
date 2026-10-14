//! GH #6936: with a FORMULA as the scope, a `log:includes` / `log:notIncludes` /
//! `log:supports` pattern given through a variable is matched as the stored formula the
//! variable is DIRECTLY bound to, as stored, when that formula holds no variable at any
//! depth (its blank nodes are its own existentials), exactly as the same formula written
//! in the rule. Every other value (a formula holding a variable, so any chained binding,
//! a non-formula value, an unbound variable), and every bound pattern with the STORE as
//! the scope, still matches as the empty pattern, as on main (GH #6943): the
//! stratification does not read a pattern given through a variable.

use sparq_reason::n3::{parser, reason_n3_terms_with_limits, ClosureLimits, Term};
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

/// A stored formula that holds a variable is not followed through the rule's bindings:
/// `?f = { :a :p ?o }` with `?o = :b` matches as the empty pattern, as on main, so
/// `log:includes` holds and `log:notIncludes` fails even against a scope that does not
/// hold `:a :p :b`.
#[test]
fn a_chained_binding_takes_the_empty_pattern_path() {
    let src = format!(
        "{} :cfg :f {{ :a :p ?o }}. :cfg :o :b.",
        doc(
            "{ :cfg :f ?f. :cfg :o ?o. { :a :p :c } log:includes ?f } => { :r :includes :chain }.
             { :cfg :f ?f. :cfg :o ?o. { :a :p :c } log:notIncludes ?f } => { :r :notIncludes :chain }.
             { :cfg :f ?f. :cfg :o ?o. { :a :p :c } log:supports ?f } => { :r :supports :chain }."
        )
    );
    let mut want = fired("includes", "chain");
    want.extend(fired("supports", "chain"));
    want.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(text(&src, NegationCycles::Reject).expect("text"), want, "{src}");
    assert_eq!(incremental(&src).expect("incremental"), want, "{src}");
}

/// The nested chain of a review: `?f = { :a :p ?o }`, `?o = { :b :q ?x }`, `?x = :c`,
/// with the scope `{ :a :p ?o }`. The stored formula holds a variable, so the pattern is
/// the empty one, as on main: `log:includes` holds and `log:notIncludes` fails, which is
/// also what the inline pattern `{ :a :p ?o }` gives against that scope.
#[test]
fn a_formula_scope_naming_the_chain_behaves_as_on_main() {
    let src = format!(
        "{} :cfg :f {{ :a :p ?o }}. :cfg :o {{ :b :q ?x }}. :cfg :x :c.",
        doc(
            "{ :cfg :f ?f. :cfg :o ?o. :cfg :x ?x. { :a :p ?o } log:includes ?f } => { :r :includes :scope }.
             { :cfg :f ?f. :cfg :o ?o. :cfg :x ?x. { :a :p ?o } log:notIncludes ?f } => { :r :notIncludes :scope }.
             { :cfg :f ?f. :cfg :o ?o. :cfg :x ?x. { :a :p ?o } log:includes { :a :p ?o } } => { :r :includes :inline }."
        )
    );
    let mut want = fired("includes", "inline");
    want.extend(fired("includes", "scope"));
    want.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(text(&src, NegationCycles::Reject).expect("text"), want, "{src}");
    assert_eq!(incremental(&src).expect("incremental"), want, "{src}");
}

/// `?f = { :a :p ?o }` with `?o` bound to a blank node: the stored formula holds a
/// variable, so nothing is substituted (the blank never becomes a pattern existential)
/// and the pattern is the empty one, as on main.
#[test]
fn a_variable_bound_to_a_blank_takes_the_empty_pattern_path() {
    let src = format!(
        "{} :cfg :f {{ :a :p ?o }}. :cfg :o _:fixed.",
        doc(
            "{ :cfg :f ?f. :cfg :o ?o. { :a :p :b } log:includes ?f } => { :r :includes :blank }.
             { :cfg :f ?f. :cfg :o ?o. { :a :p :b } log:notIncludes ?f } => { :r :notIncludes :blank }."
        )
    );
    assert_eq!(text(&src, NegationCycles::Reject).expect("text"), fired("includes", "blank"), "{src}");
    assert_eq!(incremental(&src).expect("incremental"), fired("includes", "blank"), "{src}");
}

/// A ground stored formula with a nested formula is matched as stored, the same as the
/// formula written inline.
#[test]
fn a_ground_nested_bound_pattern_is_matched_as_written_inline() {
    for (key, want_inc) in [("in", true), ("out", false)] {
        for bound in [true, false] {
            let pat = if key == "in" { "{ :a :p { :b :q :c } }" } else { "{ :a :p { :b :q :d } }" };
            let obj = if bound { "?f" } else { pat };
            let src = format!(
                "{} :cfg :{key} {pat}.",
                doc(&format!(
                    "{{ :cfg :{key} ?f. {{ :a :p {{ :b :q :c }} }} log:includes {obj} }} => {{ :r :includes :{key} }}.
                     {{ :cfg :{key} ?f. {{ :a :p {{ :b :q :c }} }} log:notIncludes {obj} }} => {{ :r :notIncludes :{key} }}."
                ))
            );
            let want = if want_inc { fired("includes", key) } else { fired("notIncludes", key) };
            assert_eq!(text(&src, NegationCycles::Reject).expect("text"), want, "{src}");
            assert_eq!(incremental(&src).expect("incremental"), want, "{src}");
        }
    }
}

/// The smallest `max_join_rows` under which `src` closes.
fn least_join_rows(src: &str) -> usize {
    (0..10_000)
        .find(|&rows| {
            let mut l = ClosureLimits::default();
            l.max_join_rows = rows;
            reason_n3_terms_with_limits(src, None, None, NegationCycles::Reject, l).is_ok()
        })
        .expect("closes within the sweep")
}

/// A bound pattern is charged to the round's `max_join_rows` allowance, one row per
/// statement it holds (nested ones included), on top of everything the same formula
/// written inline is charged: it never costs less than the inline pattern.
#[test]
fn a_bound_pattern_is_charged_at_least_as_the_inline_one() {
    let pat = "{ :a :p :b. :c :p :b. :d :p { :e :q :f } }";
    let scope = "{ :a :p :b. :c :p :b. :d :p { :e :q :f } }";
    let at = |obj: &str| {
        format!(
            "{} :cfg :big {pat}.",
            doc(&format!("{{ :cfg :big ?f. {scope} log:includes {obj} }} => {{ :r :includes :big }}."))
        )
    };
    let (bound, inline) = (at("?f"), at(pat));
    assert_eq!(text(&bound, NegationCycles::Reject).expect("text"), fired("includes", "big"));
    assert_eq!(text(&inline, NegationCycles::Reject).expect("text"), fired("includes", "big"));
    let (b, i) = (least_join_rows(&bound), least_join_rows(&inline));
    assert!(b >= i + 4, "bound {b}, inline {i}");
}
