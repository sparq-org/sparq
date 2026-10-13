//! GH #6854: the incremental counting engine behind [`MaterializedN3Graph`] evaluates
//! `string:concatenation`, `string:scrape`, `string:encodeForUri`, `log:uri` and
//! `log:equalTo` itself. It used to build their terms without the text engine's term-size
//! checkpoint and had no round or fact allowance, so the GH #6771 repro grew a term every
//! round until memory ran out.
//!
//! Each test compares the graph with the checked batch engine under the same
//! [`ClosureLimits`]: where the batch run succeeds the graph holds its closure, and where
//! it refuses the graph fails closed (an `Err` from the constructor; after a mutation, the
//! base alone with the error in `stratification_warning`). The term-size sweeps check that
//! the two agree at every limit swept, through construction, insert and delete. The round
//! and fact allowance is not swept for parity (the counting path's rounds are propagation
//! waves, not the batch engine's schedule), only checked well inside and well past it.
//!
//! The repro runs under the DEFAULT limits, so this binary is also meant to pass under a
//! bounded address space (`ulimit -v`).

use rustc_hash::FxHashSet;
use sparq_reason::n3::{reason_n3_terms_with_limits, ClosureLimits, NegationCycles, Term};
use sparq_reason::{MaterializedN3Graph, N3Mode};

const PREFIXES: &str = "@prefix : <http://ex/>.\n\
     @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
     @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n";

/// The GH #6771 repro: each round concatenates the literal with itself.
const STRING_DOUBLING: &str = "@prefix : <http://ex/>.\n\
     @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
     :a :p \"x\".\n\
     { :a :p ?s. (?s ?s) string:concatenation ?t } => { :a :p ?t }.\n";

/// The repro's rule without its seed fact.
const DOUBLING_RULE: &str = "@prefix : <http://ex/>.\n\
     @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
     { :a :p ?s. (?s ?s) string:concatenation ?t } => { :a :p ?t }.\n";

/// One rule per builtin the counting engine evaluates, each building a term from `?s`.
const PROFILES: &[(&str, &str)] = &[
    (
        "string:concatenation",
        "{ ?x :p ?s. (?s ?s ?s) string:concatenation ?t } => { ?x :q ?t }.",
    ),
    (
        "string:scrape",
        "{ ?x :p ?s. (?s \"^(.*)$\") string:scrape ?t } => { ?x :q ?t }.",
    ),
    (
        "string:encodeForUri",
        "{ ?x :p ?s. ?s string:encodeForUri ?t } => { ?x :q ?t }.",
    ),
    ("log:uri (reverse)", "{ ?x :p ?s. ?u log:uri ?s } => { ?x :q ?u }."),
    (
        "log:uri (forward)",
        "{ ?x :p ?s. ?u log:uri ?s. ?u log:uri ?t } => { ?x :r ?t }.",
    ),
    (
        "log:equalTo argument list",
        "{ ?x :p ?s. (?s ?s ?s ?s) log:equalTo (?s ?s ?s ?s) } => { ?x :q :same }.",
    ),
    (
        "string:concatenation, recursive layer",
        "{ ?x :p ?s. (?s \"/\") string:concatenation ?t. ?t string:encodeForUri ?u } => { ?x :p ?u }.",
    ),
];

fn ex(local: &str) -> Term {
    Term::Iri(format!("http://ex/{local}"))
}

fn s_lit(v: &str) -> Term {
    Term::Lit(
        v.into(),
        "http://www.w3.org/2001/XMLSchema#string".into(),
        None,
    )
}

/// The facts the sweeps assert: literals of growing length, some needing %-encoding.
fn facts() -> Vec<[Term; 3]> {
    [("a", 4), ("b", 40), ("c", 90), ("d", 160), ("e", 300)]
        .iter()
        .map(|&(subject, n)| {
            let v: String = "ab c/".chars().cycle().take(n).collect();
            [ex(subject), ex("p"), s_lit(&v)]
        })
        .collect()
}

fn serialize(facts: &FxHashSet<[Term; 3]>) -> String {
    let mut out = String::new();
    for f in facts {
        for t in f {
            match t {
                Term::Iri(i) => out.push_str(&format!("<{i}> ")),
                Term::Lit(v, _, None) => out.push_str(&format!("\"{v}\" ")),
                other => panic!("unexpected term shape in test base: {other:?}"),
            }
        }
        out.push_str(".\n");
    }
    out
}

fn limits(term_bytes: usize) -> ClosureLimits {
    let mut l = ClosureLimits::default();
    l.max_term_bytes = term_bytes;
    l
}

/// The checked batch engine over `rules` and `base`.
fn batch(
    rules: &str,
    base: &FxHashSet<[Term; 3]>,
    l: ClosureLimits,
) -> Result<FxHashSet<[Term; 3]>, String> {
    let src = format!("{rules}\n{}", serialize(base));
    reason_n3_terms_with_limits(&src, None, None, NegationCycles::Reject, l)
        .map(|c| c.facts.into_iter().collect())
}

fn assert_limit_error(what: &str, e: &str) {
    assert!(e.contains("forward closure limit exceeded"), "{what}: {e}");
}

/// The graph is in the state the batch engine gives over the same base and limits:
/// its closure, or (when the batch run refuses) the base alone and the refusal.
fn assert_matches_batch(
    g: &MaterializedN3Graph,
    rules: &str,
    base: &FxHashSet<[Term; 3]>,
    l: ClosureLimits,
    at: &str,
) {
    let held: FxHashSet<[Term; 3]> = g.closure().into_iter().collect();
    match batch(rules, base, l) {
        Ok(closure) => {
            assert_eq!(
                g.stratification_warning(),
                None,
                "{at}: graph refused, batch did not"
            );
            assert_eq!(
                held, closure,
                "{at}: closure differs from the batch engine's"
            );
        }
        Err(e) => {
            assert_limit_error(at, &e);
            let w = g.stratification_warning().unwrap_or_else(|| {
                panic!(
                    "{at}: batch refused ({e}) but the graph holds {} facts",
                    held.len()
                )
            });
            assert_limit_error(at, w);
            assert_eq!(
                &held, base,
                "{at}: a refused graph must hold the base alone"
            );
        }
    }
}

#[test]
fn the_doubling_repro_is_refused_under_the_default_limits() {
    match MaterializedN3Graph::new(STRING_DOUBLING, &[]) {
        Err(e) => {
            assert_limit_error("MaterializedN3Graph::new", &e);
            assert!(e.contains("max_term_bytes"), "{e}");
        }
        Ok(g) => panic!(
            "a closure came back for a term that grows every round: {} facts",
            g.len()
        ),
    }
}

#[test]
fn the_doubling_repro_is_refused_under_a_small_term_limit() {
    for bytes in [1 << 8, 1 << 12, 1 << 16] {
        let e = MaterializedN3Graph::with_limits(STRING_DOUBLING, &[], limits(bytes))
            .err()
            .expect("refused");
        assert_limit_error("with_limits", &e);
    }
}

#[test]
fn an_inserted_seed_fails_the_graph_closed_and_deleting_it_recovers() {
    let mut g = MaterializedN3Graph::new(DOUBLING_RULE, &[]).expect("no seed, no growth");
    assert_eq!(g.mode(), N3Mode::Counting);
    let seed = [ex("a"), ex("p"), s_lit("x")];
    let other = [ex("b"), ex("p"), s_lit("y")];
    g.insert(&[seed.clone(), other.clone()]);
    let w = g
        .stratification_warning()
        .expect("the insert passes the term limit");
    assert_limit_error("insert", w);
    assert!(w.contains("max_term_bytes"), "{w}");
    let held: FxHashSet<[Term; 3]> = g.closure().into_iter().collect();
    let base: FxHashSet<[Term; 3]> = [seed.clone(), other.clone()].into_iter().collect();
    assert_eq!(held, base, "fails closed to the base");
    assert!(
        g.fallback_reason()
            .is_some_and(|r| r.contains("ClosureLimits allowance")),
        "{:?}",
        g.fallback_reason()
    );
    // A further insert keeps it refused, and removing the seed brings counting back.
    g.insert(&[[ex("c"), ex("p"), s_lit("z")]]);
    assert!(g.stratification_warning().is_some());
    g.delete(std::slice::from_ref(&seed));
    assert_eq!(g.stratification_warning(), None);
    assert_eq!(g.mode(), N3Mode::Counting);
    assert_eq!(g.fallback_reason(), None);
    assert_eq!(g.len(), 2);
}

/// Construction, insert and delete each agree with the batch engine at every term-size
/// limit swept, for every builtin the counting engine evaluates.
#[test]
fn term_limits_agree_with_the_batch_engine_through_every_mutation() {
    let all = facts();
    let mut refused = 0;
    let mut held = 0;
    for (what, rule) in PROFILES {
        let rules = format!("{PREFIXES}{rule}\n");
        for bytes in (64..1600).step_by(23) {
            let l = limits(bytes);
            // Construction over every fact.
            let base: FxHashSet<[Term; 3]> = all.iter().cloned().collect();
            let at = format!("{what}, limit {bytes}, construction");
            match MaterializedN3Graph::with_limits(&rules, &all, l) {
                Ok(g) => {
                    held += 1;
                    assert_matches_batch(&g, &rules, &base, l, &at);
                }
                Err(e) => {
                    refused += 1;
                    assert_limit_error(&at, &e);
                    let b = batch(&rules, &base, l);
                    assert!(b.is_err(), "{at}: graph refused ({e}), batch did not");
                }
            }
            // Insert one fact at a time, then delete them in the same order.
            let mut g = MaterializedN3Graph::with_limits(&rules, &[], l).expect("empty base");
            let mut base = FxHashSet::default();
            for (i, f) in all.iter().enumerate() {
                g.insert(std::slice::from_ref(f));
                base.insert(f.clone());
                assert_matches_batch(
                    &g,
                    &rules,
                    &base,
                    l,
                    &format!("{what}, limit {bytes}, insert {i}"),
                );
            }
            for (i, f) in all.iter().enumerate() {
                g.delete(std::slice::from_ref(f));
                base.remove(f);
                assert_matches_batch(
                    &g,
                    &rules,
                    &base,
                    l,
                    &format!("{what}, limit {bytes}, delete {i}"),
                );
            }
        }
    }
    // The sweep crosses the boundary: some limits refuse, some hold.
    assert!(refused > 0 && held > 0, "refused {refused}, held {held}");
}

/// Under limits the workload stays inside, the graph counts as before (the recursive
/// profile grows without end, so no limit holds it).
#[test]
fn a_workload_inside_the_limits_keeps_counting() {
    for (what, rule) in PROFILES
        .iter()
        .filter(|(what, _)| !what.contains("recursive"))
    {
        let rules = format!("{PREFIXES}{rule}\n");
        let mut g =
            MaterializedN3Graph::with_limits(&rules, &facts(), limits(1 << 16)).expect(what);
        assert_eq!(
            g.mode(),
            N3Mode::Counting,
            "{what}: {:?}",
            g.fallback_reason()
        );
        g.delete(&facts()[..2]);
        g.insert(&facts()[..1]);
        assert_eq!(
            g.mode(),
            N3Mode::Counting,
            "{what}: {:?}",
            g.fallback_reason()
        );
        assert_eq!(g.stratification_warning(), None, "{what}");
    }
}

fn rounds(max_rounds: usize, max_facts: usize) -> ClosureLimits {
    let mut l = ClosureLimits::default();
    l.max_rounds = max_rounds;
    l.max_facts = max_facts;
    l
}

/// A chain of counted rules: three propagation waves.
const CHAIN: &str = "@prefix : <http://ex/>.\n\
     { ?x :a ?y } => { ?x :b ?y }.\n\
     { ?x :b ?y } => { ?x :c ?y }.\n";

/// A recursive layer: transitive closure.
const TRANSITIVE: &str = "@prefix : <http://ex/>.\n\
     { ?x :t ?y. ?y :t ?z } => { ?x :t ?z }.\n";

fn path(n: usize) -> Vec<[Term; 3]> {
    (0..n)
        .map(|i| [ex(&format!("n{i}")), ex("t"), ex(&format!("n{}", i + 1))])
        .collect()
}

#[test]
fn the_round_and_fact_allowance_bounds_construction() {
    let fact = [ex("s"), ex("a"), ex("o")];
    let ok = MaterializedN3Graph::with_limits(CHAIN, std::slice::from_ref(&fact), rounds(16, 1000))
        .expect("inside");
    assert_eq!(ok.mode(), N3Mode::Counting);
    assert_eq!(ok.len(), 3);
    for (what, l) in [("rounds", rounds(1, 1000)), ("facts", rounds(16, 0))] {
        let e = MaterializedN3Graph::with_limits(CHAIN, std::slice::from_ref(&fact), l)
            .err()
            .unwrap_or_else(|| panic!("{what}: past the allowance"));
        assert_limit_error(what, &e);
    }
    // A recursive layer's local fixpoint is charged too.
    let chain = path(40);
    let ok =
        MaterializedN3Graph::with_limits(TRANSITIVE, &chain, rounds(64, 10_000)).expect("inside");
    assert_eq!(ok.len(), 40 * 41 / 2);
    let e = MaterializedN3Graph::with_limits(TRANSITIVE, &chain, rounds(3, 10_000))
        .err()
        .expect("past the round allowance");
    assert_limit_error("layer rounds", &e);
}

#[test]
fn the_round_and_fact_allowance_bounds_insert_and_delete() {
    // Rounds: an insert whose propagation needs more waves than allowed fails closed.
    let mut g = MaterializedN3Graph::with_limits(CHAIN, &[], rounds(1, 1000)).expect("empty");
    let fact = [ex("s"), ex("a"), ex("o")];
    g.insert(std::slice::from_ref(&fact));
    assert_limit_error(
        "insert rounds",
        g.stratification_warning().expect("refused"),
    );
    assert_eq!(g.closure(), vec![fact.clone()]);
    g.delete(std::slice::from_ref(&fact));
    assert_eq!(g.stratification_warning(), None);
    assert!(g.is_empty());

    // Facts: a transitive insert that grows the closure past the fact allowance.
    let chain = path(10);
    let mut g = MaterializedN3Graph::with_limits(TRANSITIVE, &chain[..5], rounds(1000, 30))
        .expect("15 facts fit");
    assert_eq!(g.len(), 15);
    g.insert(&chain[5..]);
    assert_limit_error(
        "insert facts",
        g.stratification_warning().expect("55 facts do not fit"),
    );
    assert_eq!(g.len(), 10, "fails closed to the base");
    g.delete(&chain[5..]);
    assert_eq!(g.stratification_warning(), None);
    assert_eq!(g.len(), 15);
    assert_eq!(g.mode(), N3Mode::Counting);
}

/// A recursive layer feeding a counted rule: the counted wave runs after the layer's.
const LAYER_THEN_COUNTED: &str = "@prefix : <http://ex/>.\n\
     { ?x :t ?y. ?y :t ?z } => { ?x :t ?z }.\n\
     { ?x :t ?y } => { ?x :u ?y. ?y :v ?x }.\n";

fn as_set(facts: &[[Term; 3]]) -> FxHashSet<[Term; 3]> {
    facts.iter().cloned().collect()
}

/// The last wave of a propagation is checked too: a closure one fact past `max_facts` is
/// refused, and one of exactly `max_facts` facts is accepted, when it is built.
#[test]
fn a_final_wave_past_the_fact_allowance_is_refused_at_construction() {
    let fact = [ex("s"), ex("a"), ex("o")];
    // The chain closes at 3 facts, the last derived in the final wave.
    let e = MaterializedN3Graph::with_limits(CHAIN, std::slice::from_ref(&fact), rounds(16, 2))
        .err()
        .expect("3 facts do not fit in 2");
    assert_limit_error("construction, final wave", &e);
    let g = MaterializedN3Graph::with_limits(CHAIN, std::slice::from_ref(&fact), rounds(16, 3))
        .expect("exactly 3 facts fit");
    assert_eq!(g.len(), 3);
    assert_eq!(g.mode(), N3Mode::Counting);
    assert_matches_batch(
        &g,
        CHAIN,
        &as_set(&[fact]),
        rounds(16, 3),
        "construction, boundary",
    );
}

#[test]
fn a_final_wave_past_the_fact_allowance_is_refused_on_insert() {
    let fact = [ex("s"), ex("a"), ex("o")];
    let mut g = MaterializedN3Graph::with_limits(CHAIN, &[], rounds(16, 2)).expect("empty");
    g.insert(std::slice::from_ref(&fact));
    assert_limit_error(
        "insert, final wave",
        g.stratification_warning().expect("refused"),
    );
    assert_eq!(g.closure(), vec![fact.clone()], "fails closed to the base");
    assert_matches_batch(
        &g,
        CHAIN,
        &as_set(std::slice::from_ref(&fact)),
        rounds(16, 2),
        "insert",
    );

    let mut g = MaterializedN3Graph::with_limits(CHAIN, &[], rounds(16, 3)).expect("empty");
    g.insert(std::slice::from_ref(&fact));
    assert_eq!(g.stratification_warning(), None);
    assert_eq!(g.len(), 3);
    assert_eq!(g.mode(), N3Mode::Counting);
}

/// A delete that brings a refused graph back re-derives its recursive layer and the
/// counted wave after it: the closure that results is checked in full.
#[test]
fn a_layer_re_derivation_after_delete_is_checked_to_its_last_wave() {
    let chain = path(6);
    // 21 :t pairs, each with a :u and a :v copy (63 facts); after deleting n3 -> n4,
    // 6 + 3 pairs and their copies (27 facts). Every allowance below 27 must refuse.
    let after: Vec<[Term; 3]> = chain.iter().filter(|f| f != &&chain[3]).cloned().collect();
    for max_facts in 9..=28 {
        let fits = max_facts >= 27;
        let l = rounds(1000, max_facts);
        let mut g = MaterializedN3Graph::with_limits(LAYER_THEN_COUNTED, &[], l).expect("empty");
        g.insert(&chain);
        assert!(
            g.stratification_warning().is_some(),
            "63 facts do not fit in {max_facts}"
        );
        g.delete(std::slice::from_ref(&chain[3]));
        let at = format!("delete, max_facts {max_facts}");
        if fits {
            assert_eq!(g.stratification_warning(), None, "{at}");
            assert_eq!(g.len(), 27, "{at}");
            assert_eq!(g.mode(), N3Mode::Counting, "{at}");
        } else {
            assert_limit_error(
                &at,
                g.stratification_warning().expect("27 facts do not fit"),
            );
            assert_eq!(g.len(), after.len(), "{at}: fails closed to the base");
        }
        assert_matches_batch(&g, LAYER_THEN_COUNTED, &as_set(&after), l, &at);
    }
}
