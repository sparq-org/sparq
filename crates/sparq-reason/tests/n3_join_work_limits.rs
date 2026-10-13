//! GH #6938: `ClosureLimits::max_join_rows` bounds the transient work of one round of an
//! N3 forward closure: the binding rows its premise joins produce and the times its rules
//! fire. Before it, a premise of k unconstrained patterns over n facts built n^k binding
//! rows in one round, however few distinct facts it concluded.
//!
//! Each fixture's per-round charge is derived by hand in its doc comment, and each
//! boundary test checks that a closure charged exactly the allowance is accepted and one
//! row less refuses it. Every charge site has a fixture whose boundary moves without it.
//! The parity tests check that the text engine, the compiled evaluator and
//! `MaterializedN3Graph` agree at every allowance swept around the boundaries.
//!
//! Debug builds close every stratum with a naive check round, which charges each rule
//! over the whole closure again. Each fixture is built so that its largest round is the
//! same in debug and release builds.

use rustc_hash::FxHashSet;
use sparq_reason::n3::{reason_n3_terms_with_limits, ClosureLimits, NegationCycles, Term};
use sparq_reason::{MaterializedN3Graph, N3Mode};

const PREFIXES: &str = "@prefix : <http://ex/>.\n\
     @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
     @prefix list: <http://www.w3.org/2000/10/swap/list#>.\n\
     @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>.\n";

/// The size of the fixtures.
const N: usize = 20;

fn rows(max_join_rows: usize) -> ClosureLimits {
    let mut l = ClosureLimits::default();
    l.max_join_rows = max_join_rows;
    l
}

fn ex(local: &str) -> Term {
    Term::Iri(format!("http://ex/{local}"))
}

/// `:a{i} <pred> :b{i}` for `i` in `from..to`.
fn edges(pred: &str, from: usize, to: usize) -> Vec<[Term; 3]> {
    (from..to)
        .map(|i| [ex(&format!("a{i}")), ex(pred), ex(&format!("b{i}"))])
        .collect()
}

fn n3(facts: &[[Term; 3]]) -> String {
    let mut out = String::new();
    for f in facts {
        for t in f {
            match t {
                Term::Iri(i) => out.push_str(&format!("<{i}> ")),
                other => panic!("unexpected term in a fixture: {other:?}"),
            }
        }
        out.push_str(".\n");
    }
    out
}

/// A cross product: `n` `:p` facts and a rule over two unconstrained `:p` patterns that
/// concludes one fact. The text engine's first (naive) round charges `n` rows for the
/// first pattern, `n²` for the second, and `n²` firings: `n + 2n²`. Its later rounds
/// charge nothing (release) or the same again (debug's closing naive round).
const CROSS: &str = "{ ?x :p ?y. ?z :p ?w } => { :k :v :done }.";

fn cross_rows(n: usize) -> usize {
    n + 2 * n * n
}

/// Two rules, the second a cross product over what the first derives. Round 1 charges
/// `2n` (the first rule's `n` rows and `n` firings; the second finds no `:q` yet). Round 2
/// is semi-naive over the `n` new `:q` facts: once per join position, `n` delta seeds,
/// `n²` rows for the other pattern and `n²` firings, so `2n + 4n²`. Debug's closing naive
/// round charges `2n` and `n + 2n²`, less.
const CHAIN: &str = "{ ?x :p ?y } => { ?x :q ?y }.\n{ ?x :q ?y. ?z :q ?w } => { :k :v :done }.";

fn chain_rows(n: usize) -> usize {
    2 * n + 4 * n * n
}

/// A negation guard: `n` items, one blocked by `m` facts. Every round evaluates the rule
/// in full (it reads the store through a scope): `n` rows for `:kind`, `m` rows for the
/// blocked item's `log:notIncludes` clause, and `n - 1` firings.
const GUARD: &str =
    "{ ?s :kind :Item. ?SCOPE log:notIncludes { ?s :blocked ?any } } => { ?s :ok :yes }.";

fn guard_facts(n: usize, m: usize) -> Vec<[Term; 3]> {
    let mut f: Vec<[Term; 3]> = (0..n)
        .map(|i| [ex(&format!("s{i}")), ex("kind"), ex("Item")])
        .collect();
    f.extend((0..m).map(|j| [ex("s0"), ex("blocked"), ex(&format!("b{j}"))]));
    f
}

fn guard_rows(n: usize, m: usize) -> usize {
    n + m + (n - 1)
}

fn doc(rules: &str, facts: &[[Term; 3]]) -> String {
    format!("{PREFIXES}{rules}\n{}", n3(facts))
}

/// The text engine's closure of `src` under `l`.
fn text(src: &str, l: ClosureLimits) -> Result<FxHashSet<[Term; 3]>, String> {
    reason_n3_terms_with_limits(src, None, None, NegationCycles::Reject, l)
        .map(|c| c.facts.into_iter().collect())
}

fn assert_join_refusal(what: &str, r: Result<impl std::fmt::Debug, String>) {
    match r {
        Err(e) => {
            assert!(e.contains("forward closure limit exceeded"), "{what}: {e}");
            assert!(e.contains("max_join_rows"), "{what}: {e}");
        }
        Ok(c) => panic!("{what}: accepted past the join-work allowance: {c:?}"),
    }
}

/// The text engine accepts `src` with an allowance of exactly `w` rows and refuses it
/// with one less.
fn assert_text_boundary(what: &str, src: &str, w: usize) {
    if let Err(e) = text(src, rows(w)) {
        panic!("{what}: refused at exactly {w} rows: {e}");
    }
    assert_join_refusal(what, text(src, rows(w - 1)).map(|c| c.len()));
}

#[test]
fn a_cross_product_is_refused_before_its_rows_are_built() {
    // Four unconstrained patterns over 300 facts: 300^4 rows, one concluded fact. Main
    // built them until memory ran out; the allowance refuses the round within a million.
    let src = doc(
        "{ ?a :p ?b. ?c :p ?d. ?e :p ?f. ?g :p ?h } => { :k :v :done }.",
        &edges("p", 0, 300),
    );
    assert_join_refusal("text engine", text(&src, rows(1_000_000)).map(|c| c.len()));
}

#[test]
fn the_default_allowance_accepts_the_fixtures() {
    for (what, src) in [
        ("cross", doc(CROSS, &edges("p", 0, N))),
        ("chain", doc(CHAIN, &edges("p", 0, N))),
        ("guard", doc(GUARD, &guard_facts(N, 5))),
    ] {
        text(&src, ClosureLimits::default()).unwrap_or_else(|e| panic!("{what}: {e}"));
    }
}

#[test]
fn text_engine_boundary_joins_and_firings() {
    assert_text_boundary("cross", &doc(CROSS, &edges("p", 0, N)), cross_rows(N));
}

#[test]
fn text_engine_boundary_delta_seeds() {
    assert_text_boundary("chain", &doc(CHAIN, &edges("p", 0, N)), chain_rows(N));
}

#[test]
fn text_engine_boundary_negation_clause() {
    assert_text_boundary("guard", &doc(GUARD, &guard_facts(N, 7)), guard_rows(N, 7));
}

#[test]
fn text_engine_boundary_list_generator() {
    // One list of `n` members: 1 row for `:items`, `n` for the first `list:member`, `n²`
    // for the second, `n²` firings. The rule reads list cells outside its joins, so every
    // round evaluates it in full and charges the same.
    let members: Vec<String> = (0..N).map(|i| format!(":e{i}")).collect();
    let src = format!(
        "{PREFIXES}:s :items ({}).\n\
         {{ :s :items ?l. ?l list:member ?x. ?l list:member ?y }} => {{ :k :v :done }}.",
        members.join(" ")
    );
    assert_text_boundary("list:member", &src, 1 + N + 2 * N * N);
}

#[test]
fn text_engine_boundary_virtual_list_access() {
    // `n` facts with a list object: `n` rows for `:items`, `n` for the virtual
    // `rdf:first` over each list term, `n` firings; every round alike.
    let facts: String = (0..N)
        .map(|i| format!(":s{i} :items (:e{i} :f{i}).\n"))
        .collect();
    let src =
        format!("{PREFIXES}{facts}{{ ?s :items ?l. ?l rdf:first ?x }} => {{ ?s :first ?x }}.");
    assert_text_boundary("rdf:first", &src, 3 * N);
}

#[test]
fn text_engine_boundary_backward_proofs() {
    // `:anc` has no facts; each round proves it through the backward rule: `n` rows for
    // the backward rule's premise, `n` proofs as rows of the forward premise, `n`
    // firings. A premise a backward rule can prove is evaluated in full every round.
    let src = doc(
        "{ ?x :anc ?y } <= { ?x :p ?y }.\n{ ?x :anc ?y } => { ?x :found ?y }.",
        &edges("p", 0, N),
    );
    assert_text_boundary("backward", &src, 3 * N);
}

#[test]
fn text_engine_boundary_scope_matches() {
    // `?S log:includes` over the store: `n` rows for `:p`; per row, the clause's `n` rows
    // (its matches, charged once as its joins build them); `n²` firings. Every round
    // alike.
    let src = doc(
        "{ ?x :p ?y. ?S log:includes { ?z :p ?w } } => { :k :v :done }.",
        &edges("p", 0, N),
    );
    assert_text_boundary("log:includes", &src, N + 2 * N * N);
}

#[test]
fn text_engine_boundary_formula_scope_matches() {
    // `log:includes` over a written formula of `m` statements: `n` rows for `:p`; per row,
    // `m` matches, each charged as the containment search finds it; `n·m` firings. Every
    // round alike.
    const M: usize = 7;
    let scope: String = (0..M).map(|j| format!(":a :q :b{j}. ")).collect();
    let src = doc(
        &format!("{{ ?x :p ?y. {{ {scope}}} log:includes {{ :a :q ?z }} }} => {{ :k :v :done }}."),
        &edges("p", 0, N),
    );
    assert_text_boundary("formula scope", &src, N + 2 * N * M);
}

#[test]
fn text_engine_boundary_transitivity_fast_path() {
    // `n` disjoint two-edge chains `:a{i} :t :b{i} :t :c{i}` under the transitivity rule
    // (its linear fast path). Round 1 charges, per chain, one forward firing (the first
    // edge extended by the generator edge after it) and one backward firing (the second
    // edge extending the path ending at its start): `2n`. Round 2 charges nothing; the
    // closing naive round charges the same `2n`.
    let facts: String = (0..N)
        .map(|i| format!(":a{i} :t :b{i}. :b{i} :t :c{i}.\n"))
        .collect();
    let src = format!("{PREFIXES}{facts}{{ ?x :t ?y. ?y :t ?z }} => {{ ?x :t ?z }}.");
    assert_text_boundary("transitivity", &src, 2 * N);
}

/// The graph is in the state the batch engine gives over the same base and limits: its
/// closure, or (when the batch run refuses) the base alone and the refusal.
fn assert_matches_batch(
    g: &MaterializedN3Graph,
    rules: &str,
    base: &[[Term; 3]],
    l: ClosureLimits,
    at: &str,
) {
    let held: FxHashSet<[Term; 3]> = g.closure().into_iter().collect();
    match text(&doc(rules, base), l) {
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
            assert!(e.contains("max_join_rows"), "{at}: {e}");
            let w = g.stratification_warning().unwrap_or_else(|| {
                panic!(
                    "{at}: batch refused ({e}) but the graph holds {} facts",
                    held.len()
                )
            });
            assert!(w.contains("max_join_rows"), "{at}: {w}");
            let base: FxHashSet<[Term; 3]> = base.iter().cloned().collect();
            assert_eq!(held, base, "{at}: a refused graph must hold the base alone");
        }
    }
}

/// `MaterializedN3Graph::with_limits` over `rules` and `base`, or the base alone with the
/// refusal (the state a refused mutation leaves): a refused constructor is checked here
/// against the batch engine directly.
fn graph_matches_batch(
    rules: &str,
    base: &[[Term; 3]],
    l: ClosureLimits,
    at: &str,
) -> Option<MaterializedN3Graph> {
    match MaterializedN3Graph::with_limits(&format!("{PREFIXES}{rules}"), base, l) {
        Ok(g) => {
            assert_matches_batch(&g, rules, base, l, at);
            Some(g)
        }
        Err(e) => {
            assert!(e.contains("max_join_rows"), "{at}: {e}");
            assert_join_refusal(at, text(&doc(rules, base), l).map(|c| c.len()));
            None
        }
    }
}

#[test]
fn counting_engine_boundary_at_construction() {
    // The counting engine's construction is one wave over every base fact. CROSS: per
    // `:p` fact seeding the first pattern, 1 seed row, `n` rows for the second pattern and
    // `n` firings; seeding the second pattern, 1 row and nothing before it (the first
    // pattern reads the store without the wave): `n(1 + 2n) + n = 2n + 2n²`.
    let base = edges("p", 0, N);
    let counting = 2 * N + 2 * N * N;
    let g = graph_matches_batch(CROSS, &base, rows(counting), "at the counting charge")
        .expect("accepted");
    assert_eq!(g.mode(), N3Mode::Counting);
    // One row less: the counting path refuses and the graph rebuilds through the batch
    // engine, whose round fits (`n + 2n²`), so it holds the batch closure.
    let g = graph_matches_batch(
        CROSS,
        &base,
        rows(counting - 1),
        "one under the counting charge",
    )
    .expect("accepted");
    assert_eq!(g.mode(), N3Mode::Fallback);
    assert!(
        g.fallback_reason()
            .is_some_and(|r| r.contains("max_join_rows")),
        "{:?}",
        g.fallback_reason()
    );
    // At and under the batch engine's round, both refuse.
    assert!(graph_matches_batch(CROSS, &base, rows(cross_rows(N)), "at the batch round").is_some());
    assert!(graph_matches_batch(
        CROSS,
        &base,
        rows(cross_rows(N) - 1),
        "under the batch round"
    )
    .is_none());

    // GUARD charges the counting wave exactly what the batch round charges: each `:kind`
    // fact seeds 1 row, the blocked item's guard `m` rows, and `n - 1` items fire.
    let base = guard_facts(N, 7);
    let w = guard_rows(N, 7);
    let g = graph_matches_batch(GUARD, &base, rows(w), "guard at its charge").expect("accepted");
    assert_eq!(g.mode(), N3Mode::Counting);
    assert!(graph_matches_batch(GUARD, &base, rows(w - 1), "guard one under").is_none());
}

#[test]
fn counting_engine_boundary_in_a_recursive_layer() {
    // `n` disjoint two-edge chains under the transitivity rule, which the counting engine
    // keeps as a recursive layer with its own local fixpoint. Its first local round seeds
    // each pattern with each edge: per chain, the first edge seeding the first pattern
    // finds the second edge (3 rows and firings), the second edge seeding the second
    // pattern finds the first (3), and the other two seeds find nothing (1 each): 8. The
    // second local round charges 2 per chain, against a fresh allowance. The batch engine
    // charges `2n` (its transitivity fast path), so one row under `8n` the graph rebuilds
    // through it and still holds its closure.
    const TRANS: &str = "{ ?x :t ?y. ?y :t ?z } => { ?x :t ?z }.";
    let base: Vec<[Term; 3]> = (0..N)
        .flat_map(|i| {
            let (a, b, c) = (
                ex(&format!("a{i}")),
                ex(&format!("b{i}")),
                ex(&format!("c{i}")),
            );
            [[a, ex("t"), b.clone()], [b, ex("t"), c]]
        })
        .collect();
    let g =
        graph_matches_batch(TRANS, &base, rows(8 * N), "at the layer's charge").expect("accepted");
    assert_eq!(g.mode(), N3Mode::Counting);
    let g = graph_matches_batch(TRANS, &base, rows(8 * N - 1), "one under").expect("accepted");
    assert_eq!(g.mode(), N3Mode::Fallback);
    assert!(graph_matches_batch(TRANS, &base, rows(2 * N - 1), "under the batch charge").is_none());
}

#[test]
fn counting_engine_layer_rounds_leave_the_wave_allowance_alone() {
    // One construction wave over a recursive layer (`n` two-edge chains under the
    // transitivity rule: local rounds of `8n` and `2n` rows, see the test above) and `m`
    // facts for a counted rule (1 seed row and 1 firing each: `2m`). Each local round of
    // the layer has its own allowance, apart from the wave's, so at `8n` both fit and the
    // graph keeps counting. Were the local rounds charged to the wave's allowance, the
    // counted rule would find only `8n - 2n` left and the graph would fall back.
    const M: usize = 70;
    let rules = "{ ?x :t ?y. ?y :t ?z } => { ?x :t ?z }.\n{ ?x :p ?y } => { ?x :q ?y }.";
    let mut base: Vec<[Term; 3]> = (0..N)
        .flat_map(|i| {
            let (a, b, c) = (
                ex(&format!("a{i}")),
                ex(&format!("b{i}")),
                ex(&format!("c{i}")),
            );
            [[a, ex("t"), b.clone()], [b, ex("t"), c]]
        })
        .collect();
    base.extend(edges("p", 0, M));
    let l = rows(8 * N);
    assert!(
        2 * M <= 8 * N && 2 * M > 8 * N - 2 * N,
        "the fixture's premise"
    );
    let g = MaterializedN3Graph::with_limits(&format!("{PREFIXES}{rules}"), &base, l)
        .expect("accepted without falling back");
    assert_eq!(g.mode(), N3Mode::Counting, "{:?}", g.fallback_reason());
    assert_eq!(g.stratification_warning(), None);
    let held: FxHashSet<[Term; 3]> = g.closure().into_iter().collect();
    let full = text(&doc(rules, &base), ClosureLimits::default()).expect("batch closure");
    assert_eq!(held, full);
    // A from-scratch batch round charges `2n + 2m` and refuses: the documented divergence.
    assert_join_refusal("batch", text(&doc(rules, &base), l).map(|c| c.len()));
}

#[test]
fn counting_engine_insert_and_delete() {
    let base = edges("p", 0, N);
    // Exactly the counting construction's charge (see the construction test).
    let l = rows(2 * N + 2 * N * N);
    let mut g = MaterializedN3Graph::with_limits(&format!("{PREFIXES}{CROSS}"), &base, l)
        .expect("accepted");
    assert_eq!(g.mode(), N3Mode::Counting);

    // Ten new facts in one insert: a wave of `10(1 + 2(n + 10)) + 10(1 + 2n)` rows, past
    // the allowance, so the graph rebuilds through the batch engine, which refuses (its
    // round over 30 facts charges `30 + 2·30²`): the graph fails closed.
    let more = edges("p", N, N + 10);
    g.insert(&more);
    let all: Vec<[Term; 3]> = base.iter().chain(&more).cloned().collect();
    assert_matches_batch(&g, CROSS, &all, l, "after the refused insert");
    assert!(g.stratification_warning().is_some());

    // Deleting them brings the graph back within the allowance, counting again.
    g.delete(&more);
    assert_matches_batch(&g, CROSS, &base, l, "after the delete");
    assert_eq!(g.mode(), N3Mode::Counting);

    // One new fact: a wave of `(1 + 2(n + 1)) + (1 + 2n) = 4n + 4` rows fits, so the
    // counting path keeps it. A from-scratch batch run over the `n + 1` facts charges
    // `(n + 1) + 2(n + 1)²` and refuses: the documented divergence (the graph never
    // refuses a closure the batch engine accepts, but a mutation whose wave fits is kept
    // even when a from-scratch batch round would not fit).
    let one = edges("p", N, N + 1);
    g.insert(&one);
    assert_eq!(g.mode(), N3Mode::Counting);
    assert_eq!(g.stratification_warning(), None);
    assert!(g.contains(&[ex("k"), ex("v"), ex("done")]));
    let all: Vec<[Term; 3]> = base.iter().chain(&one).cloned().collect();
    assert_join_refusal(
        "batch over n + 1 facts",
        text(&doc(CROSS, &all), l).map(|c| c.len()),
    );

    // Deleting it is the same wave in reverse, and leaves the batch closure.
    g.delete(&one);
    assert_eq!(g.mode(), N3Mode::Counting);
    assert_matches_batch(&g, CROSS, &base, l, "after deleting the one fact");
}

#[test]
fn counting_engine_matches_batch_through_mutations_at_every_allowance() {
    // Swept across both engines' charges for the guard fixture, through construction,
    // inserts and deletes of blocking facts: at every allowance the graph holds the batch
    // closure or fails closed exactly when the batch engine refuses.
    let base = guard_facts(N, 3);
    let extra: Vec<[Term; 3]> = (3..9)
        .map(|j| [ex("s0"), ex("blocked"), ex(&format!("b{j}"))])
        .collect();
    for w in guard_rows(N, 0)..=guard_rows(N, 9) + 1 {
        let l = rows(w);
        let at = format!("max_join_rows {w}");
        let Ok(mut g) = MaterializedN3Graph::with_limits(&format!("{PREFIXES}{GUARD}"), &base, l)
        else {
            assert_join_refusal(&at, text(&doc(GUARD, &base), l).map(|c| c.len()));
            continue;
        };
        let mut now = base.clone();
        assert_matches_batch(&g, GUARD, &now, l, &at);
        for f in &extra {
            g.insert(std::slice::from_ref(f));
            now.push(f.clone());
            assert_matches_batch(&g, GUARD, &now, l, &format!("{at}, insert {f:?}"));
        }
        for f in &extra {
            g.delete(std::slice::from_ref(f));
            now.retain(|x| x != f);
            assert_matches_batch(&g, GUARD, &now, l, &format!("{at}, delete {f:?}"));
        }
    }
}

#[cfg(feature = "compiled-rules")]
mod compiled {
    use super::*;
    use sparq_core::dict::{Dict, Id};
    use sparq_reason::n3::compiled::{compile, eval_with_limits};
    use sparq_reason::reason_n3_with_limits;

    fn compiled(src: &str, l: ClosureLimits) -> Result<FxHashSet<[Id; 3]>, String> {
        let mut dict = Dict::new();
        let rules = compile(src)?;
        eval_with_limits(&mut dict, &[], &rules, l).map(|c| c.into_iter().collect())
    }

    #[test]
    fn a_cross_product_is_refused_before_its_rows_are_built() {
        let src = doc(
            "{ ?a :p ?b. ?c :p ?d. ?e :p ?f. ?g :p ?h } => { :k :v :done }.",
            &edges("p", 0, 300),
        );
        assert_join_refusal("compiled", compiled(&src, rows(1_000_000)).map(|c| c.len()));
    }

    #[test]
    fn compiled_boundaries() {
        for (what, src, w) in [
            ("cross", doc(CROSS, &edges("p", 0, N)), cross_rows(N)),
            ("chain", doc(CHAIN, &edges("p", 0, N)), chain_rows(N)),
            ("guard", doc(GUARD, &guard_facts(N, 7)), guard_rows(N, 7)),
        ] {
            if let Err(e) = compiled(&src, rows(w)) {
                panic!("{what}: refused at exactly {w} rows: {e}");
            }
            assert_join_refusal(what, compiled(&src, rows(w - 1)).map(|c| c.len()));
        }
    }

    #[test]
    fn the_text_and_compiled_engines_agree_at_every_allowance() {
        for (what, src, w) in [
            ("cross", doc(CROSS, &edges("p", 0, N)), cross_rows(N)),
            ("chain", doc(CHAIN, &edges("p", 0, N)), chain_rows(N)),
            ("guard", doc(GUARD, &guard_facts(N, 7)), guard_rows(N, 7)),
        ] {
            for at in w - 2 * N..=w + 2 * N {
                let mut dict = Dict::new();
                let t = reason_n3_with_limits(&mut dict, &src, NegationCycles::Reject, rows(at));
                let rules = compile(&src).expect("compiles");
                let c = eval_with_limits(&mut dict, &[], &rules, rows(at));
                match (t, c) {
                    (Ok(t), Ok(c)) => assert_eq!(
                        t.into_iter().collect::<FxHashSet<_>>(),
                        c.into_iter().collect::<FxHashSet<_>>(),
                        "{what} at {at}"
                    ),
                    (Err(t), Err(c)) => assert_eq!(t, c, "{what} at {at}"),
                    (t, c) => panic!(
                        "{what} at {at}: text {:?}, compiled {:?}",
                        t.map(|r| r.len()),
                        c.map(|r| r.len())
                    ),
                }
            }
        }
    }
}
