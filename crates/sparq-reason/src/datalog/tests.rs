//! [FABLE-5] sq-6tykl.3 — Datalog acceptance suite: parser + stratification-checker
//! unit tests, hand-computed eval fixtures (exact expected sets — a mutation flips
//! them red), and the DIFFERENTIAL harness against the independent naive oracle
//! (`super::oracle`) on fixed programs × seed-randomised graphs.

use super::{eval, oracle, parse_program, souffle, stratify};
use rustc_hash::FxHashSet;
use sparq_core::dict::{Dict, Id};

const EX: &str = "http://ex/";
const P: &str = "@prefix ex: <http://ex/> .\n";

fn iri(d: &mut Dict, local: &str) -> Id {
    d.intern_iri(&format!("{EX}{local}"))
}

fn int(d: &mut Dict, n: u64) -> Id {
    d.intern_lit(
        &n.to_string(),
        "http://www.w3.org/2001/XMLSchema#integer",
        None,
    )
}

fn s(d: &mut Dict, v: &str) -> Id {
    d.intern_lit(v, "http://www.w3.org/2001/XMLSchema#string", None)
}

/// Evaluate and return ONLY the derived facts (closure minus inputs), as a set.
fn derived(d: &mut Dict, facts: &[[Id; 3]], src: &str) -> FxHashSet<[Id; 3]> {
    let p = parse_program(d, src).expect("parse");
    let closure = eval(d, facts, &p).expect("stratifiable");
    let inputs: FxHashSet<[Id; 3]> = facts.iter().copied().collect();
    closure
        .into_iter()
        .filter(|f| !inputs.contains(f))
        .collect()
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

#[test]
fn parse_basics_and_a_sugar() {
    let mut d = Dict::new();
    let p = parse_program(
        &mut d,
        &format!("{P}[?x, a, ex:Mortal] :- [?x, a, ex:Man] .\n# comment\n"),
    )
    .unwrap();
    assert_eq!(p.n_rules(), 1);
    // `a` expanded to rdf:type in both the head and the body atom.
    let ty = d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    assert_eq!(p.rules[0].head[0].pred, Some(ty));
    assert_eq!(p.rules[0].positive[0].pred, Some(ty));
}

#[test]
fn parse_accepts_variable_predicate() {
    let mut d = Dict::new();
    let p = parse_program(&mut d, &format!("{P}[?x, ?p, ?y] :- [?x, ?p, ?y] .")).unwrap();
    assert_eq!(p.n_rules(), 1);
}

#[test]
fn parse_accepts_not_exists_group_with_period_separators() {
    let mut d = Dict::new();
    let p = parse_program(
        &mut d,
        &format!(
            "{P}[?x, ex:q, \"y\"] :- [?x, a, ex:Node], \
             NOT EXISTS {{ [?x, ex:p, ?z] . [?z, ex:r, ?x] . }} ."
        ),
    )
    .unwrap();
    assert_eq!(p.n_rules(), 1);
}

#[test]
fn parse_rejects_unknown_aggregate_functions() {
    let mut d = Dict::new();
    let e = parse_program(
        &mut d,
        &format!("{P}[?x, ex:t, ?s] :- AGGREGATE([?x, ex:v, ?y] ON ?x BIND MEDIAN(?y) AS ?s) ."),
    )
    .unwrap_err();
    assert!(e.contains("MEDIAN") && e.contains("unknown"), "{e}");
}

#[test]
fn parse_rejects_unsafe_head_variable() {
    let mut d = Dict::new();
    let e = parse_program(&mut d, &format!("{P}[?x, ex:q, ?z] :- [?x, ex:p, ?y] .")).unwrap_err();
    assert!(e.contains("unsafe rule") && e.contains("?z"), "{e}");
}

#[test]
fn parse_rejects_head_var_bound_only_by_not() {
    let mut d = Dict::new();
    let e = parse_program(
        &mut d,
        &format!("{P}[?y, ex:q, \"y\"] :- [?x, a, ex:N], NOT [?x, ex:p, ?y] ."),
    )
    .unwrap_err();
    assert!(e.contains("unsafe rule"), "{e}");
}

#[test]
fn parse_rejects_aggregate_local_capture() {
    let mut d = Dict::new();
    // ?y occurs both inside the aggregate body (non-ON) and as an outer atom var.
    let e = parse_program(
        &mut d,
        &format!(
            "{P}[?x, ex:t, ?c] :- [?x, ex:p, ?y], \
             AGGREGATE([?x, ex:v, ?y] ON ?x BIND COUNT(?y) AS ?c) ."
        ),
    )
    .unwrap_err();
    assert!(e.contains("aggregate-local"), "{e}");
}

#[test]
fn parse_rejects_on_var_missing_from_body_and_undeclared_prefix() {
    let mut d = Dict::new();
    let e = parse_program(
        &mut d,
        &format!("{P}[?z, ex:t, ?c] :- AGGREGATE([?x, ex:v, ?y] ON ?z BIND COUNT(?y) AS ?c) ."),
    )
    .unwrap_err();
    assert!(e.contains("does not occur"), "{e}");
    let e2 = parse_program(&mut d, "[?x, foo:p, ?y] :- [?x, foo:q, ?y] .").unwrap_err();
    assert!(e2.contains("undeclared prefix"), "{e2}");
}

#[test]
fn parse_rejects_duplicate_aggregate_outputs_and_unbound_filter_var() {
    let mut d = Dict::new();
    let e = parse_program(
        &mut d,
        &format!(
            "{P}[?x, ex:t, ?c] :- AGGREGATE([?x, ex:v, ?y] ON ?x BIND COUNT(?y) AS ?c), \
             AGGREGATE([?x, ex:w, ?z] ON ?x BIND COUNT(?z) AS ?c) ."
        ),
    )
    .unwrap_err();
    assert!(e.contains("same output variable"), "{e}");
    let e2 = parse_program(
        &mut d,
        &format!("{P}[?x, ex:q, ?y] :- [?x, ex:p, ?y], FILTER(?nope > 1) ."),
    )
    .unwrap_err();
    assert!(e2.contains("FILTER variable"), "{e2}");
}

#[test]
fn parse_rejects_fresh_violation_of_aggregate_output() {
    let mut d = Dict::new();
    let e = parse_program(
        &mut d,
        &format!(
            "{P}[?x, ex:t, ?c] :- [?x, ex:p, ?c], \
             AGGREGATE([?x, ex:v, ?y] ON ?x BIND COUNT(?y) AS ?c) ."
        ),
    )
    .unwrap_err();
    assert!(e.contains("must be fresh"), "{e}");
}

// ---------------------------------------------------------------------------
// Stratification checker
// ---------------------------------------------------------------------------

#[test]
fn stratify_pure_positive_recursion_is_one_stratum() {
    let mut d = Dict::new();
    let p = parse_program(
        &mut d,
        &format!("{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n[?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:edge, ?z] ."),
    )
    .unwrap();
    let s = stratify(&d, &p).unwrap();
    assert_eq!(s.n_strata(), 1);
}

#[test]
fn stratify_naf_over_derived_needs_a_higher_stratum() {
    let mut d = Dict::new();
    let p = parse_program(
        &mut d,
        &format!(
            "{P}[?x, ex:reach, \"y\"] :- [?x, ex:seed, \"y\"] .\n\
             [?y, ex:reach, \"y\"] :- [?x, ex:reach, \"y\"], [?x, ex:edge, ?y] .\n\
             [?x, ex:unreach, \"y\"] :- [?x, a, ex:Node], NOT [?x, ex:reach, \"y\"] ."
        ),
    )
    .unwrap();
    let s = stratify(&d, &p).unwrap();
    // reach: stratum 0 wrt seed/edge (EDB); unreach must sit strictly above reach.
    assert_eq!(s.n_strata(), 2);
    assert_eq!(s.rule_stratum, vec![0, 0, 1]);
}

#[test]
fn stratify_rejects_negation_cycle_naming_the_predicate() {
    let mut d = Dict::new();
    // win(X) :- move(X,Y), NOT win(Y).
    let p = parse_program(
        &mut d,
        &format!("{P}[?x, ex:win, \"y\"] :- [?x, ex:move, ?y], NOT [?y, ex:win, \"y\"] ."),
    )
    .unwrap();
    let e = stratify(&d, &p).unwrap_err();
    assert!(
        e.contains("NOT stratifiable") && e.contains("http://ex/win"),
        "{e}"
    );
}

#[test]
fn stratify_rejects_aggregation_cycle() {
    let mut d = Dict::new();
    // p counts itself: no stratified model.
    let p = parse_program(
        &mut d,
        &format!("{P}[?x, ex:p, ?c] :- AGGREGATE([?x, ex:p, ?y] ON ?x BIND COUNT(?y) AS ?c) ."),
    )
    .unwrap();
    assert!(stratify(&d, &p).is_err());
}

#[test]
fn stratify_chains_strata_through_aggregates() {
    let mut d = Dict::new();
    let p = parse_program(
        &mut d,
        &format!(
            "{P}[?x, ex:deg, ?c] :- AGGREGATE([?x, ex:edge, ?y] ON ?x BIND COUNT(?y) AS ?c) .\n\
             [?x, ex:isolated, \"y\"] :- [?x, a, ex:Node], NOT [?x, ex:deg, ?c] ."
        ),
    )
    .unwrap();
    let s = stratify(&d, &p).unwrap();
    // edge (EDB, 0) → deg (1) → isolated (2).
    assert_eq!(s.n_strata(), 3);
    assert_eq!(s.rule_stratum, vec![1, 2]);
}

#[test]
fn stratify_co_head_predicates_share_a_stratum() {
    let mut d = Dict::new();
    let p = parse_program(
        &mut d,
        &format!(
            "{P}[?x, ex:p, \"y\"], [?x, ex:q, \"y\"] :- [?x, a, ex:N], NOT [?x, ex:r, \"y\"] .\n\
             [?x, ex:r, \"y\"] :- [?x, ex:base, \"y\"] ."
        ),
    )
    .unwrap();
    let s = stratify(&d, &p).unwrap();
    assert_eq!(s.rule_stratum, vec![1, 0]);
}

#[test]
fn stratify_is_class_granular_for_rdf_type() {
    let mut d = Dict::new();
    // Deriving `a ex:Hub` and negating `a ex:Hub` from a DIFFERENT class head must
    // stratify (rdf:type is not one monolithic relation) …
    let p = parse_program(
        &mut d,
        &format!(
            "{P}[?x, ex:deg, ?c] :- AGGREGATE([?x, ex:edge, ?y] ON ?x BIND COUNT(?y) AS ?c) .\n\
             [?x, a, ex:Hub] :- [?x, ex:deg, ?c], FILTER(?c >= 2) .\n\
             [?x, a, ex:Leaf] :- [?x, a, ex:Node], NOT [?x, a, ex:Hub] ."
        ),
    )
    .unwrap();
    let s = stratify(&d, &p).unwrap();
    assert_eq!(s.rule_stratum, vec![1, 1, 2]);
    // … while a genuine cycle through one class is still rejected, named as a class.
    let p2 = parse_program(
        &mut d,
        &format!("{P}[?x, a, ex:Win] :- [?x, ex:move, ?y], NOT [?y, a, ex:Win] ."),
    )
    .unwrap();
    let e = stratify(&d, &p2).unwrap_err();
    assert!(e.contains("class") && e.contains("http://ex/Win"), "{e}");
}

#[test]
fn stratify_variable_class_reads_sit_above_every_class() {
    let mut d = Dict::new();
    // `NOT [?x, a, ?c]` (variable class) must sit above the derived ex:Hub class.
    let p = parse_program(
        &mut d,
        &format!(
            "{P}[?x, a, ex:Hub] :- [?x, ex:big, \"y\"] .\n\
             [?x, ex:untyped, \"y\"] :- [?x, ex:thing, \"y\"], NOT [?x, a, ?c] ."
        ),
    )
    .unwrap();
    let s = stratify(&d, &p).unwrap();
    assert_eq!(s.rule_stratum, vec![0, 1]);
    // And a variable-class HEAD feeding a negated class is rejected.
    let p2 = parse_program(
        &mut d,
        &format!("{P}[?x, a, ?c] :- [?x, ex:says, ?c], NOT [?x, a, ex:Banned] ."),
    )
    .unwrap();
    assert!(stratify(&d, &p2).is_err());
}

// ---------------------------------------------------------------------------
// Evaluation fixtures (hand-computed expected sets)
// ---------------------------------------------------------------------------

#[test]
fn eval_transitive_closure() {
    let mut d = Dict::new();
    let (a, b, c, edge, reach) = (
        iri(&mut d, "a"),
        iri(&mut d, "b"),
        iri(&mut d, "c"),
        iri(&mut d, "edge"),
        iri(&mut d, "reach"),
    );
    let facts = vec![[a, edge, b], [b, edge, c]];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n\
             [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:edge, ?z] ."
        ),
    );
    let want: FxHashSet<[Id; 3]> = [[a, reach, b], [b, reach, c], [a, reach, c]]
        .into_iter()
        .collect();
    assert_eq!(got, want);
}

#[test]
fn eval_naf_absence_check_across_strata() {
    let mut d = Dict::new();
    let (a, b, c, ty, node, edge, seed, reach, unreach, y) = (
        iri(&mut d, "a"),
        iri(&mut d, "b"),
        iri(&mut d, "c"),
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        iri(&mut d, "Node"),
        iri(&mut d, "edge"),
        iri(&mut d, "seed"),
        iri(&mut d, "reach"),
        iri(&mut d, "unreach"),
        s(&mut d, "y"),
    );
    // a → b, c disconnected; a seeded.
    let facts = vec![
        [a, ty, node],
        [b, ty, node],
        [c, ty, node],
        [a, edge, b],
        [a, seed, y],
    ];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?x, ex:reach, \"y\"] :- [?x, ex:seed, \"y\"] .\n\
             [?y, ex:reach, \"y\"] :- [?x, ex:reach, \"y\"], [?x, ex:edge, ?y] .\n\
             [?x, ex:unreach, \"y\"] :- [?x, a, ex:Node], NOT [?x, ex:reach, \"y\"] ."
        ),
    );
    let want: FxHashSet<[Id; 3]> = [[a, reach, y], [b, reach, y], [c, unreach, y]]
        .into_iter()
        .collect();
    assert_eq!(got, want);
}

/// [GPT-5.6] A grouped NOT is one existential conjunction, not a list of
/// independently-negated atoms: both patterns have matches, but no single `?z`
/// joins them, so the rule must fire.
#[test]
fn naf_conjunction_absence() {
    let mut d = Dict::new();
    let (a, x, y, ty, node, p, q, absent, yes) = (
        iri(&mut d, "a"),
        iri(&mut d, "x"),
        iri(&mut d, "y"),
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        iri(&mut d, "Node"),
        iri(&mut d, "p"),
        iri(&mut d, "q"),
        iri(&mut d, "absent"),
        s(&mut d, "y"),
    );
    let facts = vec![[a, ty, node], [a, p, x], [a, q, y]];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?x, ex:absent, \"y\"] :- [?x, a, ex:Node], \
             NOT {{ [?x, ex:p, ?z], [?x, ex:q, ?z] }} ."
        ),
    );
    assert!(
        got.contains(&[a, absent, yes]),
        "grouped NOT must test the joint match"
    );
}

#[test]
fn eval_count_per_group_and_threshold_filter() {
    let mut d = Dict::new();
    let (g1, g2, m1, m2, m3, member, deg, ty, hub) = (
        iri(&mut d, "g1"),
        iri(&mut d, "g2"),
        iri(&mut d, "m1"),
        iri(&mut d, "m2"),
        iri(&mut d, "m3"),
        iri(&mut d, "member"),
        iri(&mut d, "deg"),
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        iri(&mut d, "Hub"),
    );
    let facts = vec![
        [g1, member, m1],
        [g1, member, m2],
        [g1, member, m3],
        [g2, member, m1],
    ];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?g, ex:deg, ?c] :- AGGREGATE([?g, ex:member, ?m] ON ?g BIND COUNT(?m) AS ?c) .\n\
             [?g, a, ex:Hub] :- [?g, ex:deg, ?c], FILTER(?c >= 2) ."
        ),
    );
    let three = int(&mut d, 3);
    let one = int(&mut d, 1);
    let want: FxHashSet<[Id; 3]> = [[g1, deg, three], [g2, deg, one], [g1, ty, hub]]
        .into_iter()
        .collect();
    assert_eq!(got, want);
}

/// [GPT-5.6] Legacy COUNT counts distinct full body tuples, whereas DISTINCT
/// projects the selected value before de-duplication.
#[test]
fn count_distinct_vs_count() {
    let mut d = Dict::new();
    let (g, member, tag, count, count_distinct, v, red, blue) = (
        iri(&mut d, "g"),
        iri(&mut d, "member"),
        iri(&mut d, "tag"),
        iri(&mut d, "count"),
        iri(&mut d, "countDistinct"),
        iri(&mut d, "v"),
        iri(&mut d, "red"),
        iri(&mut d, "blue"),
    );
    let facts = vec![[g, member, v], [v, tag, red], [v, tag, blue]];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?g, ex:count, ?c] :- AGGREGATE([?g, ex:member, ?v], [?v, ex:tag, ?t] \
             ON ?g BIND COUNT(?v) AS ?c) .\n\
             [?g, ex:countDistinct, ?c] :- AGGREGATE([?g, ex:member, ?v], [?v, ex:tag, ?t] \
             ON ?g BIND COUNT(DISTINCT ?v) AS ?c) ."
        ),
    );
    let (one, two) = (int(&mut d, 1), int(&mut d, 2));
    assert!(got.contains(&[g, count, two]));
    assert!(got.contains(&[g, count_distinct, one]));
}

#[test]
fn filter_double_comparison() {
    let mut d = Dict::new();
    let (a, value, big, yes) = (
        iri(&mut d, "a"),
        iri(&mut d, "value"),
        iri(&mut d, "big"),
        s(&mut d, "y"),
    );
    let two_half = d.intern_lit("2.5", "http://www.w3.org/2001/XMLSchema#double", None);
    let facts = vec![[a, value, two_half]];
    let got = derived(
        &mut d,
        &facts,
        &format!("{P}[?x, ex:big, \"y\"] :- [?x, ex:value, ?v], FILTER(?v > 2) ."),
    );
    assert!(got.contains(&[a, big, yes]));
}

#[test]
fn filter_nan_fails_row() {
    let mut d = Dict::new();
    let (a, value) = (iri(&mut d, "a"), iri(&mut d, "value"));
    let nan = d.intern_lit("NaN", "http://www.w3.org/2001/XMLSchema#double", None);
    let facts = vec![[a, value, nan]];
    let got = derived(
        &mut d,
        &facts,
        &format!("{P}[?x, ex:eq, \"y\"] :- [?x, ex:value, ?v], FILTER(?v = ?v) ."),
    );
    assert!(got.is_empty(), "NaN comparison is a failed FILTER row");
}

#[test]
fn variable_predicate_closure() {
    let mut d = Dict::new();
    let (a, b, predicate, value, likes, observed) = (
        iri(&mut d, "a"),
        iri(&mut d, "b"),
        iri(&mut d, "predicate"),
        iri(&mut d, "value"),
        iri(&mut d, "likes"),
        iri(&mut d, "observed"),
    );
    let facts = vec![[a, predicate, likes], [a, value, b]];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?s, ?p, ?o] :- [?s, ex:predicate, ?p], [?s, ex:value, ?o] .\n\
             [?p, ex:observed, ?o] :- [ex:a, ?p, ?o] ."
        ),
    );
    assert!(
        got.contains(&[a, likes, b]),
        "variable head predicate must derive"
    );
    assert!(
        got.contains(&[likes, observed, b]),
        "variable body predicate must match"
    );
}

/// [GPT-5.6] When a different positive atom consumes the current delta, the
/// variable-predicate sibling must still scan the full, older store.
#[test]
fn variable_predicate_joins_store_union_on_later_delta() {
    let mut d = Dict::new();
    let (a, b, x, p, seed, trigger, seen, yes) = (
        iri(&mut d, "a"),
        iri(&mut d, "b"),
        iri(&mut d, "x"),
        iri(&mut d, "p"),
        iri(&mut d, "seed"),
        iri(&mut d, "trigger"),
        iri(&mut d, "seen"),
        s(&mut d, "y"),
    );
    let facts = vec![[a, p, b], [x, seed, a]];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?x, ex:trigger, \"y\"] :- [?x, ex:seed, ?a] .\n\
             [?p, ex:seen, ?o] :- [?x, ex:trigger, \"y\"], [ex:a, ?p, ?o] ."
        ),
    );
    assert!(got.contains(&[p, seen, b]));
    assert!(got.contains(&[x, trigger, yes]));
}

#[test]
fn variable_head_predicate_must_be_an_iri() {
    let mut d = Dict::new();
    let (a, b, predicate, value) = (
        iri(&mut d, "a"),
        iri(&mut d, "b"),
        iri(&mut d, "predicate"),
        iri(&mut d, "value"),
    );
    let not_an_iri = s(&mut d, "not a predicate");
    let facts = vec![[a, predicate, not_an_iri], [a, value, b]];
    let got = derived(
        &mut d,
        &facts,
        &format!("{P}[?s, ?p, ?o] :- [?s, ex:predicate, ?p], [?s, ex:value, ?o] ."),
    );
    assert!(
        got.is_empty(),
        "an RDF predicate position must resolve to an IRI"
    );
}

#[test]
fn variable_predicate_forces_conservative_strata() {
    let mut d = Dict::new();
    let p = parse_program(
        &mut d,
        &format!("{P}[?x, ?p, \"y\"] :- [?x, ex:names, ?p], NOT [?x, ex:block, \"y\"] ."),
    )
    .unwrap();
    let e = stratify(&d, &p).unwrap_err();
    assert!(
        e.contains("NOT stratifiable") && e.contains("variable-predicate"),
        "{e}"
    );

    // The top node includes rdf:type's class-granular dependency nodes too.
    let class_cycle = parse_program(
        &mut d,
        &format!("{P}[?x, ?p, \"y\"] :- [?x, ex:names, ?p], NOT [?x, a, ex:Banned] ."),
    )
    .unwrap();
    assert!(stratify(&d, &class_cycle).is_err());
}

/// [GPT-5.6] Incremental maintenance must preserve the same top-node dependency:
/// an insert under any predicate can affect a variable-predicate body.
#[test]
fn variable_predicate_incremental_matches_from_scratch() {
    let mut d = Dict::new();
    let src = format!("{P}[?p, ex:seen, ?o] :- [ex:a, ?p, ?o] .");
    let program = parse_program(&mut d, &src).unwrap();
    let (a, p, b, seen) = (
        iri(&mut d, "a"),
        iri(&mut d, "p"),
        iri(&mut d, "b"),
        iri(&mut d, "seen"),
    );
    let fact = [a, p, b];
    let mut maintained = MaterializedProgram::new(&mut d, &[], program.clone()).unwrap();
    maintained.insert(&mut d, &[fact]);
    let incremental: FxHashSet<_> = maintained.closure().into_iter().collect();
    let fresh: FxHashSet<_> = eval(&mut d, &[fact], &program)
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(incremental, fresh);
    assert!(incremental.contains(&[p, seen, b]));
}

/// [GPT-5.6] A removed asserted fact that a variable-predicate head still derives
/// must transfer ownership into the stratum instead of disappearing. This is the
/// `head_any` deletion/re-ownership branch, not the variable-body `read_any` branch.
#[test]
fn variable_head_incremental_delete_reowns_derived_fact() {
    let mut d = Dict::new();
    let src = format!("{P}[?s, ?p, ?o] :- [?s, ex:predicate, ?p], [?s, ex:value, ?o] .");
    let program = parse_program(&mut d, &src).unwrap();
    let (a, b, p, predicate, value) = (
        iri(&mut d, "a"),
        iri(&mut d, "b"),
        iri(&mut d, "p"),
        iri(&mut d, "predicate"),
        iri(&mut d, "value"),
    );
    let asserted = [a, p, b];
    let supports = [[a, predicate, p], [a, value, b]];
    let mut base = supports.to_vec();
    base.push(asserted);
    let mut maintained = MaterializedProgram::new(&mut d, &base, program.clone()).unwrap();
    assert_eq!(maintained.delete(&mut d, &[asserted]), 0);
    let incremental: FxHashSet<_> = maintained.closure().into_iter().collect();
    let fresh: FxHashSet<_> = eval(&mut d, &supports, &program)
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(incremental, fresh);
    assert!(incremental.contains(&asserted));
}

#[test]
fn grouped_naf_incremental_boundary_matches_from_scratch() {
    let mut d = Dict::new();
    let src = format!(
        "{P}[?x, ex:absent, \"y\"] :- [?x, a, ex:Node], \
         NOT {{ [?x, ex:p, ?z], [?x, ex:q, ?z] }} ."
    );
    let program = parse_program(&mut d, &src).unwrap();
    let (a, x, y, ty, node, p, q) = (
        iri(&mut d, "a"),
        iri(&mut d, "x"),
        iri(&mut d, "y"),
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        iri(&mut d, "Node"),
        iri(&mut d, "p"),
        iri(&mut d, "q"),
    );
    let base = vec![[a, ty, node], [a, p, x], [a, q, y]];
    let inserted = [a, q, x];
    let mut maintained = MaterializedProgram::new(&mut d, &base, program.clone()).unwrap();
    maintained.insert(&mut d, &[inserted]);
    let mut updated = base;
    updated.push(inserted);
    let incremental: FxHashSet<_> = maintained.closure().into_iter().collect();
    let fresh: FxHashSet<_> = eval(&mut d, &updated, &program)
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(incremental, fresh);
}

#[test]
fn count_distinct_incremental_boundary_matches_from_scratch() {
    let mut d = Dict::new();
    let src = format!(
        "{P}[?g, ex:n, ?c] :- AGGREGATE([?g, ex:member, ?v], [?v, ex:tag, ?t] \
         ON ?g BIND COUNT(DISTINCT ?v) AS ?c) ."
    );
    let program = parse_program(&mut d, &src).unwrap();
    let (g, v, v2, member, tag, n, red, blue) = (
        iri(&mut d, "g"),
        iri(&mut d, "v"),
        iri(&mut d, "v2"),
        iri(&mut d, "member"),
        iri(&mut d, "tag"),
        iri(&mut d, "n"),
        iri(&mut d, "red"),
        iri(&mut d, "blue"),
    );
    // v2 is already a member but has no tag, so it does not occur in the aggregate
    // solution set until the update. The projected DISTINCT count must change 1 -> 2.
    let base = vec![[g, member, v], [g, member, v2], [v, tag, red]];
    let inserted = [v2, tag, blue];
    let mut maintained = MaterializedProgram::new(&mut d, &base, program.clone()).unwrap();
    let (one, two) = (int(&mut d, 1), int(&mut d, 2));
    assert!(maintained.contains(&[g, n, one]));
    maintained.insert(&mut d, &[inserted]);
    let mut updated = base;
    updated.push(inserted);
    let incremental: FxHashSet<_> = maintained.closure().into_iter().collect();
    let fresh: FxHashSet<_> = eval(&mut d, &updated, &program)
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(incremental, fresh);
    assert!(!incremental.contains(&[g, n, one]));
    assert!(incremental.contains(&[g, n, two]));
}

#[test]
fn sum_per_group() {
    let mut d = Dict::new();
    let (g1, g2, value, total) = (
        iri(&mut d, "g1"),
        iri(&mut d, "g2"),
        iri(&mut d, "value"),
        iri(&mut d, "total"),
    );
    let (one, two, five) = (int(&mut d, 1), int(&mut d, 2), int(&mut d, 5));
    let facts = vec![[g1, value, one], [g1, value, two], [g2, value, five]];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?g, ex:total, ?s] :- AGGREGATE([?g, ex:value, ?v] ON ?g BIND SUM(?v) AS ?s) ."
        ),
    );
    let (three, five_out) = (int(&mut d, 3), int(&mut d, 5));
    assert_eq!(
        got,
        [[g1, total, three], [g2, total, five_out]]
            .into_iter()
            .collect()
    );
}

#[test]
fn min_max_preserve_input_term() {
    let mut d = Dict::new();
    let (g, value, min_p, max_p) = (
        iri(&mut d, "g"),
        iri(&mut d, "value"),
        iri(&mut d, "min"),
        iri(&mut d, "max"),
    );
    let low = d.intern_lit("1", "http://www.w3.org/2001/XMLSchema#long", None);
    let high = d.intern_lit("9", "http://www.w3.org/2001/XMLSchema#short", None);
    let facts = vec![[g, value, high], [g, value, low]];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?g, ex:min, ?v] :- AGGREGATE([?g, ex:value, ?x] ON ?g BIND MIN(?x) AS ?v) .\n\
         [?g, ex:max, ?v] :- AGGREGATE([?g, ex:value, ?x] ON ?g BIND MAX(?x) AS ?v) ."
        ),
    );
    assert_eq!(
        got,
        [[g, min_p, low], [g, max_p, high]].into_iter().collect()
    );
}

#[test]
fn avg_of_integers_is_decimal() {
    let mut d = Dict::new();
    let (g, value, avg) = (iri(&mut d, "g"), iri(&mut d, "value"), iri(&mut d, "avg"));
    let (one, two) = (int(&mut d, 1), int(&mut d, 2));
    let facts = vec![[g, value, one], [g, value, two]];
    let got = derived(
        &mut d,
        &facts,
        &format!("{P}[?g, ex:avg, ?a] :- AGGREGATE([?g, ex:value, ?v] ON ?g BIND AVG(?v) AS ?a) ."),
    );
    let one_half = d.intern_lit("1.5", "http://www.w3.org/2001/XMLSchema#decimal", None);
    assert_eq!(got, [[g, avg, one_half]].into_iter().collect());
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] sq-r2nor — the DECIDED float/double semantics for the numeric
// aggregates: the whole XSD numeric tower is in scope (no fail-closed reject),
// SUM/AVG promote per XPath, and the fold order + NaN/-0.0 rule are pinned so the
// closure stays a function of the completed lower strata.
// ---------------------------------------------------------------------------

const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
const XSD_FLOAT: &str = "http://www.w3.org/2001/XMLSchema#float";

fn dbl(d: &mut Dict, lex: &str) -> Id {
    d.intern_lit(lex, XSD_DOUBLE, None)
}

/// A `(term id, numeric value)` group in the shape `fold_numeric_group` consumes.
fn group(d: &mut Dict, ids: &[Id]) -> Vec<(Id, sparq_substrate::numeric::Num)> {
    ids.iter()
        .map(|&id| {
            (
                id,
                super::numeric_value(d, id).expect("test fixture is numeric"),
            )
        })
        .collect()
}

#[test]
fn sum_and_avg_promote_a_double_operand() {
    let mut d = Dict::new();
    let (g, value, total, avg) = (
        iri(&mut d, "g"),
        iri(&mut d, "value"),
        iri(&mut d, "total"),
        iri(&mut d, "avg"),
    );
    let (one, half) = (int(&mut d, 1), dbl(&mut d, "0.5"));
    let facts = vec![[g, value, one], [g, value, half]];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?g, ex:total, ?s] :- AGGREGATE([?g, ex:value, ?v] ON ?g BIND SUM(?v) AS ?s) .\n\
             [?g, ex:avg, ?a] :- AGGREGATE([?g, ex:value, ?v] ON ?g BIND AVG(?v) AS ?a) ."
        ),
    );
    // XPath promotion: an xsd:double operand makes both results xsd:double, in the
    // canonical mantissa-E-exponent lexical.
    let (sum_out, avg_out) = (dbl(&mut d, "1.5E0"), dbl(&mut d, "7.5E-1"));
    assert_eq!(
        got,
        [[g, total, sum_out], [g, avg, avg_out]]
            .into_iter()
            .collect()
    );
}

#[test]
fn min_max_totalise_nan_below_negative_infinity() {
    let mut d = Dict::new();
    let (g, value, min_p, max_p) = (
        iri(&mut d, "g"),
        iri(&mut d, "value"),
        iri(&mut d, "min"),
        iri(&mut d, "max"),
    );
    let (nan, ninf, one) = (
        dbl(&mut d, "NaN"),
        dbl(&mut d, "-INF"),
        dbl(&mut d, "1.0E0"),
    );
    let facts = vec![[g, value, one], [g, value, nan], [g, value, ninf]];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?g, ex:min, ?v] :- AGGREGATE([?g, ex:value, ?x] ON ?g BIND MIN(?x) AS ?v) .\n\
             [?g, ex:max, ?v] :- AGGREGATE([?g, ex:value, ?x] ON ?g BIND MAX(?x) AS ?v) ."
        ),
    );
    // NaN is NOT a row-level failure for MIN/MAX (unlike FILTER, which uses the
    // relational comparison): the total order puts it below -INF.
    assert_eq!(
        got,
        [[g, min_p, nan], [g, max_p, one]].into_iter().collect()
    );
}

#[test]
fn float_sum_fold_is_independent_of_group_order() {
    let mut d = Dict::new();
    // Classic non-associative catastrophic-cancellation set: an unsorted left-to-right
    // fold gives 2, 3 or 4 depending on where the big pair lands, so a hash-order fold
    // would make the derived SUM depend on the derivation order.
    let ids = [
        dbl(&mut d, "1.0E16"),
        dbl(&mut d, "1.0E0"),
        dbl(&mut d, "-1.0E16"),
        dbl(&mut d, "2.0E0"),
    ];
    let mut base = group(&mut d, &ids);
    let want = eval::fold_numeric_group(&mut d, super::AggFunc::Sum, &mut base)
        .expect("a non-empty group folds");
    for rot in 1..ids.len() {
        let mut rotated: Vec<Id> = ids[rot..].to_vec();
        rotated.extend_from_slice(&ids[..rot]);
        let mut values = group(&mut d, &rotated);
        assert_eq!(
            eval::fold_numeric_group(&mut d, super::AggFunc::Sum, &mut values),
            Some(want),
            "rotation by {} changed the SUM",
            rot
        );
    }
    let mut reversed = group(&mut d, &[ids[3], ids[2], ids[1], ids[0]]);
    assert_eq!(
        eval::fold_numeric_group(&mut d, super::AggFunc::Sum, &mut reversed),
        Some(want)
    );
    // The pinned order is ascending value: -1e16, 1, 2, 1e16.
    assert_eq!(want, dbl(&mut d, "2.0E0"));
}

/// Fold a two-term value tie in a FRESH dictionary that interns the pair in the given
/// order, and return the emitted RDF TERM. Returning the term, not the id, is the point:
/// ids are interning-order artefacts, so an assertion phrased in ids cannot see a
/// tie-break that moved with the interning order.
fn tie_term(first: (&str, &str), second: (&str, &str), func: super::AggFunc) -> oxrdf::Term {
    let mut d = Dict::new();
    let ids = [
        d.intern_lit(first.0, first.1, None),
        d.intern_lit(second.0, second.1, None),
    ];
    let mut values = group(&mut d, &ids);
    let id = eval::fold_numeric_group(&mut d, func, &mut values).expect("a non-empty group folds");
    d.term(id)
}

fn lit_term((lex, datatype): (&str, &str)) -> oxrdf::Term {
    oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
        lex,
        oxrdf::NamedNode::new_unchecked(datatype),
    ))
}

#[test]
fn min_max_break_a_signed_zero_tie_by_term_content() {
    // +0.0 and -0.0 are EQUAL under the total order, so the emitted term is decided by
    // the tie-break — the same one for MIN and for MAX. Interning the pair in OPPOSITE
    // orders gives the two terms opposite dictionary ids, which is exactly the freedom a
    // derivation-order change has; a content tie-break must ignore it.
    let (pos, neg) = (("0.0E0", XSD_DOUBLE), ("-0.0E0", XSD_DOUBLE));
    for func in [super::AggFunc::Min, super::AggFunc::Max] {
        assert_eq!(
            tie_term(pos, neg, func),
            tie_term(neg, pos, func),
            "{:?} moved with the interning order",
            func
        );
        // "-0.0E0" sorts before "0.0E0" as a lexical form, for both functions.
        assert_eq!(tie_term(pos, neg, func), lit_term(neg));
    }
}

#[test]
fn min_max_break_a_cross_tier_value_tie_by_term_content() {
    // "1.0"^^xsd:decimal and "1.0E0"^^xsd:double are distinct TERMS of equal VALUE
    // (neither is an inlined id, so their relative ids follow the interning order).
    let (exact, inexact) = (("1.0", XSD_DECIMAL), ("1.0E0", XSD_DOUBLE));
    for func in [super::AggFunc::Min, super::AggFunc::Max] {
        assert_eq!(
            tie_term(exact, inexact, func),
            tie_term(inexact, exact, func),
            "{:?} moved with the interning order",
            func
        );
        // xsd:decimal orders before xsd:double as a datatype IRI.
        assert_eq!(tie_term(exact, inexact, func), lit_term(exact));
    }
}

#[test]
fn sum_of_equal_valued_operands_is_id_order_independent() {
    // A tie between EQUAL-valued operands of different tiers is not free for SUM: the
    // fold promotes as it goes, so the tie order decides where the accumulator leaves
    // the float tier. Here `(1 + 2^24) as f32` rounds the 1 away, while `1 + 2^24` in
    // the double tier keeps it — so the two tie orders differ in the last unit.
    let one = ("1.0E0", XSD_FLOAT);
    let (as_float, as_double) = (("1.6777216E7", XSD_FLOAT), ("1.6777216E7", XSD_DOUBLE));
    let fold = |order: [(&str, &str); 3]| {
        let mut dict = Dict::new();
        let ids: Vec<Id> = order
            .iter()
            .map(|&(lex, dt)| dict.intern_lit(lex, dt, None))
            .collect();
        let mut values = group(&mut dict, &ids);
        let id = eval::fold_numeric_group(&mut dict, super::AggFunc::Sum, &mut values)
            .expect("a non-empty group folds");
        dict.term(id)
    };
    let want = fold([one, as_float, as_double]);
    for order in [
        [one, as_double, as_float],
        [as_float, as_double, one],
        [as_double, as_float, one],
        [as_float, one, as_double],
    ] {
        assert_eq!(fold(order), want, "the SUM moved with the interning order");
    }
    // xsd:double orders before xsd:float, so the accumulator promotes at the tie and
    // the 1 survives: 1 + 2^24 + 2^24 == 33554433, not 33554432.
    assert_eq!(want, lit_term(("3.3554433E7", XSD_DOUBLE)));
}

/// Derived facts as RDF TERM triples, so two runs over different dictionaries can be
/// compared (ids cannot be).
fn derived_terms(d: &mut Dict, facts: &[[Id; 3]], src: &str) -> FxHashSet<[String; 3]> {
    derived(d, facts, src)
        .into_iter()
        .map(|f| f.map(|id| d.term(id).to_string()))
        .collect()
}

#[test]
fn a_tie_over_minted_terms_is_independent_of_the_minting_order() {
    // The tied operands are MINTED by an earlier stratum, so their ids come out of the
    // aggregate's hash-map iteration order — the concrete way a derivation-order change
    // reassigns ids. Here the same flip is forced by pre-interning one of the two: with
    // `seed`, "0.0E0" is interned BEFORE the facts and holds the smaller id; without it,
    // "0.0E0" is minted afterwards and holds the larger. The closure must not move.
    let src = format!(
        "{P}[?g, ex:total, ?s] :- AGGREGATE([?g, ex:value, ?v] ON ?g BIND SUM(?v) AS ?s) .\n\
         [ex:world, ex:min, ?m] :- AGGREGATE([?g, ex:total, ?t] BIND MIN(?t) AS ?m) .\n\
         [ex:world, ex:max, ?m] :- AGGREGATE([?g, ex:total, ?t] BIND MAX(?t) AS ?m) ."
    );
    let run = |seed: bool| {
        let mut d = Dict::new();
        if seed {
            dbl(&mut d, "0.0E0");
        }
        let (g1, g2, value) = (iri(&mut d, "g1"), iri(&mut d, "g2"), iri(&mut d, "value"));
        // g1 sums to +0.0 (a MINTED term); g2's one-element sum is -0.0. They are equal
        // under the total order and distinct as terms.
        let facts = vec![
            [g1, value, dbl(&mut d, "1.0E0")],
            [g1, value, dbl(&mut d, "-1.0E0")],
            [g2, value, dbl(&mut d, "-0.0E0")],
        ];
        derived_terms(&mut d, &facts, &src)
    };
    assert_eq!(run(false), run(true));
}

#[test]
fn empty_group_yields_no_row_for_sum() {
    let mut d = Dict::new();
    let got = derived(
        &mut d,
        &[],
        &format!(
            "{P}[ex:world, ex:total, ?s] :- AGGREGATE([?x, ex:value, ?v] BIND SUM(?v) AS ?s) ."
        ),
    );
    assert!(got.is_empty());
}

#[test]
fn non_numeric_value_fails_row() {
    let mut d = Dict::new();
    let (g, value, total) = (iri(&mut d, "g"), iri(&mut d, "value"), iri(&mut d, "total"));
    let (two, text) = (int(&mut d, 2), s(&mut d, "not numeric"));
    let facts = vec![[g, value, two], [g, value, text]];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?g, ex:total, ?s] :- AGGREGATE([?g, ex:value, ?v] ON ?g BIND SUM(?v) AS ?s) ."
        ),
    );
    assert_eq!(got, [[g, total, two]].into_iter().collect());
}

#[test]
fn eval_global_count_without_on() {
    let mut d = Dict::new();
    let (a, b, ty, node, total, w) = (
        iri(&mut d, "a"),
        iri(&mut d, "b"),
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        iri(&mut d, "Node"),
        iri(&mut d, "total"),
        iri(&mut d, "world"),
    );
    let facts = vec![[a, ty, node], [b, ty, node]];
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[ex:world, ex:total, ?c] :- AGGREGATE([?x, a, ex:Node] BIND COUNT(?x) AS ?c) ."
        ),
    );
    let two = int(&mut d, 2);
    let want: FxHashSet<[Id; 3]> = [[w, total, two]].into_iter().collect();
    assert_eq!(got, want);
}

#[test]
fn eval_count_over_derived_predicate() {
    let mut d = Dict::new();
    let (a, b, c, edge, nreach, w) = (
        iri(&mut d, "a"),
        iri(&mut d, "b"),
        iri(&mut d, "c"),
        iri(&mut d, "edge"),
        iri(&mut d, "nreach"),
        iri(&mut d, "world"),
    );
    let facts = vec![[a, edge, b], [b, edge, c]];
    // reach is derived (stratum 0); the count reads it from stratum 1.
    let got = derived(
        &mut d,
        &facts,
        &format!(
            "{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n\
             [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:edge, ?z] .\n\
             [ex:world, ex:nreach, ?c] :- AGGREGATE([?x, ex:reach, ?y] BIND COUNT(?x) AS ?c) ."
        ),
    );
    // reach = {(a,b),(b,c),(a,c)} → 3 distinct matches.
    let three = int(&mut d, 3);
    assert!(
        got.contains(&[w, nreach, three]),
        "count over DERIVED reach"
    );
    assert_eq!(got.len(), 4); // 3 reach facts + the count fact
}

#[test]
fn eval_naf_repeated_wildcard_requires_equal_terms() {
    let mut d = Dict::new();
    let (a, b, p, ok, ty, n, y) = (
        iri(&mut d, "a"),
        iri(&mut d, "b"),
        iri(&mut d, "p"),
        iri(&mut d, "ok"),
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        iri(&mut d, "N"),
        s(&mut d, "y"),
    );
    // No self-loop exists: NOT [?z, ex:p, ?z] holds even though (a,p,b) exists.
    let facts = vec![[a, ty, n], [a, p, b]];
    let got = derived(
        &mut d,
        &facts,
        &format!("{P}[?x, ex:ok, \"y\"] :- [?x, a, ex:N], NOT [?z, ex:p, ?z] ."),
    );
    let want: FxHashSet<[Id; 3]> = [[a, ok, y]].into_iter().collect();
    assert_eq!(got, want);
    // Add the self-loop: the wildcard NAF now fails and nothing derives.
    let facts2 = vec![[a, ty, n], [a, p, b], [b, p, b]];
    let got2 = derived(
        &mut d,
        &facts2,
        &format!("{P}[?x, ex:ok, \"y\"] :- [?x, a, ex:N], NOT [?z, ex:p, ?z] ."),
    );
    assert!(got2.is_empty());
}

#[test]
fn eval_multi_head_rule_derives_both() {
    let mut d = Dict::new();
    let (a, b, p, q, r) = (
        iri(&mut d, "a"),
        iri(&mut d, "b"),
        iri(&mut d, "p"),
        iri(&mut d, "q"),
        iri(&mut d, "r"),
    );
    let facts = vec![[a, p, b]];
    let got = derived(
        &mut d,
        &facts,
        &format!("{P}[?x, ex:q, ?y], [?y, ex:r, ?x] :- [?x, ex:p, ?y] ."),
    );
    let want: FxHashSet<[Id; 3]> = [[a, q, b], [b, r, a]].into_iter().collect();
    assert_eq!(got, want);
}

#[test]
fn eval_rejects_non_stratifiable_program() {
    let mut d = Dict::new();
    let p = parse_program(
        &mut d,
        &format!("{P}[?x, ex:win, \"y\"] :- [?x, ex:move, ?y], NOT [?y, ex:win, \"y\"] ."),
    )
    .unwrap();
    assert!(eval(&mut d, &[], &p).is_err());
}

#[test]
fn eval_filter_is_fail_closed_on_non_numeric_operands() {
    let mut d = Dict::new();
    let (a, p, b) = (iri(&mut d, "a"), iri(&mut d, "p"), iri(&mut d, "b"));
    // ?y is an IRI — the numeric FILTER must fail the row, deriving nothing.
    let facts = vec![[a, p, b]];
    let got = derived(
        &mut d,
        &facts,
        &format!("{P}[?x, ex:q, ?y] :- [?x, ex:p, ?y], FILTER(?y > 0) ."),
    );
    assert!(got.is_empty());
}

// ---------------------------------------------------------------------------
// Differential vs the independent naive oracle
// ---------------------------------------------------------------------------

/// Deterministic LCG (no rand dep; fixed seeds → reproducible graphs).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }
}

/// Random node/edge facts over `n` nodes: every node typed, ~`e` random edges.
fn random_graph(d: &mut Dict, seed: u64, n: u64, e: u64) -> Vec<[Id; 3]> {
    let ty = d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let node = iri(d, "Node");
    let edge = iri(d, "edge");
    let seed_p = iri(d, "seed");
    let weight = iri(d, "weight");
    let y = s(d, "y");
    let mut rng = Lcg(seed.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(1));
    let nodes: Vec<Id> = (0..n).map(|i| iri(d, &format!("n{i}"))).collect();
    let mut facts: Vec<[Id; 3]> = nodes.iter().map(|&x| [x, ty, node]).collect();
    for &x in &nodes {
        let v = int(d, rng.next(7) + 1);
        facts.push([x, weight, v]);
    }
    for _ in 0..e {
        let (i, j) = (rng.next(n) as usize, rng.next(n) as usize);
        facts.push([nodes[i], edge, nodes[j]]);
    }
    // One seeded node so reachability programs have a source.
    facts.push([nodes[rng.next(n) as usize], seed_p, y]);
    facts
}

/// The differential body: engine closure == oracle closure, as sets.
fn assert_differential(src: &str, seeds: std::ops::Range<u64>) {
    for seed in seeds {
        let mut d = Dict::new();
        let program = parse_program(&mut d, src).expect("parse");
        let strat = stratify(&d, &program).expect("stratifiable");
        let facts = random_graph(&mut d, seed, 6, 9);
        let engine: FxHashSet<[Id; 3]> = eval(&mut d, &facts, &program)
            .unwrap()
            .into_iter()
            .collect();
        let reference = oracle::eval_naive(&mut d, &facts, &program, &strat);
        assert_eq!(
            engine, reference,
            "engine/oracle divergence: seed {seed} program:\n{src}"
        );
    }
}

/// Engine/oracle equality plus a per-seed witness that a variable predicate was
/// emitted by a head and then consumed by a variable-predicate body in a later round.
fn assert_variable_predicate_differential(src: &str, seeds: std::ops::Range<u64>) {
    for seed in seeds {
        let mut d = Dict::new();
        let program = parse_program(&mut d, src).expect("parse");
        let strat = stratify(&d, &program).expect("stratifiable");
        let facts = random_graph(&mut d, seed, 6, 9);
        let edge = iri(&mut d, "edge");
        let node = iri(&mut d, "Node");
        let observed = iri(&mut d, "observed");
        let &[subject, _, object] = facts
            .iter()
            .find(|fact| fact[1] == edge)
            .expect("random fixture always has an edge");
        let engine: FxHashSet<[Id; 3]> = eval(&mut d, &facts, &program)
            .unwrap()
            .into_iter()
            .collect();
        let reference = oracle::eval_naive(&mut d, &facts, &program, &strat);
        assert_eq!(
            engine, reference,
            "engine/oracle divergence: seed {seed} program:\n{src}"
        );
        assert!(
            engine.contains(&[subject, node, object]),
            "seed {seed}: variable head did not emit the dynamic ex:Node predicate"
        );
        assert!(
            engine.contains(&[node, observed, object]),
            "seed {seed}: variable body did not consume the dynamic-head fact"
        );
    }
}

#[test]
fn differential_recursion_plus_naf() {
    assert_differential(
        &format!(
            "{P}[?x, ex:reach, \"y\"] :- [?x, ex:seed, \"y\"] .\n\
             [?y, ex:reach, \"y\"] :- [?x, ex:reach, \"y\"], [?x, ex:edge, ?y] .\n\
             [?x, ex:unreach, \"y\"] :- [?x, a, ex:Node], NOT [?x, ex:reach, \"y\"] ."
        ),
        0..25,
    );
}

#[test]
fn differential_count_threshold() {
    assert_differential(
        &format!(
            "{P}[?x, ex:deg, ?c] :- AGGREGATE([?x, ex:edge, ?y] ON ?x BIND COUNT(?y) AS ?c) .\n\
             [?x, a, ex:Hub] :- [?x, ex:deg, ?c], FILTER(?c >= 2) .\n\
             [?x, a, ex:Leaf] :- [?x, a, ex:Node], NOT [?x, a, ex:Hub] ."
        ),
        0..25,
    );
}

#[test]
fn differential_sum_threshold() {
    assert_differential(
        &format!(
            "{P}[ex:world, ex:total, ?s] :- AGGREGATE([?x, ex:weight, ?v] BIND SUM(?v) AS ?s) .\n\
         [ex:world, ex:large, \"y\"] :- [ex:world, ex:total, ?s], FILTER(?s > 10) ."
        ),
        0..25,
    );
}

#[test]
fn differential_min_max_avg() {
    assert_differential(
        &format!(
            "{P}[ex:world, ex:min, ?v] :- AGGREGATE([?x, ex:weight, ?n] BIND MIN(?n) AS ?v) .\n\
         [ex:world, ex:max, ?v] :- AGGREGATE([?x, ex:weight, ?n] BIND MAX(?n) AS ?v) .\n\
         [ex:world, ex:avg, ?v] :- AGGREGATE([?x, ex:weight, ?n] BIND AVG(?n) AS ?v) ."
        ),
        0..25,
    );
}

#[test]
fn differential_count_over_derived_with_wildcard_naf() {
    assert_differential(
        &format!(
            "{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n\
             [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:edge, ?z] .\n\
             [?x, ex:fan, ?c] :- AGGREGATE([?x, ex:reach, ?y] ON ?x BIND COUNT(?y) AS ?c) .\n\
             [ex:world, ex:quiet, \"y\"] :- [ex:world, a, ex:Nothing], NOT [?z, ex:edge, ?z] .\n\
             [?x, ex:big, \"y\"] :- [?x, ex:fan, ?c], FILTER(?c > 3) ."
        ),
        0..25,
    );
}

#[test]
fn differential_multi_head_and_global_count() {
    assert_differential(
        &format!(
            "{P}[?x, ex:out, ?y], [?y, ex:in, ?x] :- [?x, ex:edge, ?y] .\n\
             [ex:world, ex:edges, ?c] :- AGGREGATE([?x, ex:out, ?y] BIND COUNT(?x) AS ?c) ."
        ),
        0..25,
    );
}

/// [GPT-5.6] Independent-oracle arm for grouped NAF. The shared `?y` makes this
/// specifically a conjunction join, not two standalone absence checks.
#[test]
fn differential_naf_conjunction() {
    assert_differential(
        &format!(
            "{P}[?x, ex:noCycle, \"y\"] :- [?x, a, ex:Node], \
             NOT {{ [?x, ex:edge, ?y], [?y, ex:edge, ?x] }} ."
        ),
        0..25,
    );
}

/// [GPT-5.6] Independent-oracle arm for projected DISTINCT aggregation.
#[test]
fn differential_count_distinct() {
    assert_differential(
        &format!(
            "{P}[ex:world, ex:uniqueSources, ?c] :- \
             AGGREGATE([?x, ex:edge, ?y], [?x, ex:weight, ?w] \
                       BIND COUNT(DISTINCT ?x) AS ?c) ."
        ),
        0..25,
    );
}

/// [GPT-5.6] Independent-oracle arm for variable predicates in both body and
/// head positions.
#[test]
fn differential_variable_predicates() {
    assert_variable_predicate_differential(
        &format!(
            "{P}[?x, ?p, ?y] :- [?x, ex:edge, ?y], [?x, a, ?p] .\n\
             [?p, ex:observed, ?y] :- [?x, ?p, ?y] ."
        ),
        0..25,
    );
}

// ---------------------------------------------------------------------------
// Semi-naive vs naive (sq-8sve7): SET-identical closures + strictly less work
// ---------------------------------------------------------------------------

/// Three-way differential on one (program, facts) input: the semi-naive engine,
/// the same engine forced to full (naive) re-runs, and the independent oracle
/// must produce the SAME closure set.
fn assert_semi_naive_equals_naive(src: &str, facts_of: &dyn Fn(&mut Dict) -> Vec<[Id; 3]>) {
    let mut d = Dict::new();
    let program = parse_program(&mut d, src).expect("parse");
    let strat = stratify(&d, &program).expect("stratifiable");
    let facts = facts_of(&mut d);
    let mut semi_stats = eval::EvalStats::default();
    let semi: FxHashSet<[Id; 3]> =
        eval::eval_stratified_with_stats(&mut d, &facts, &program, &strat, &mut semi_stats, false)
            .into_iter()
            .collect();
    let mut naive_stats = eval::EvalStats::default();
    let naive: FxHashSet<[Id; 3]> =
        eval::eval_stratified_with_stats(&mut d, &facts, &program, &strat, &mut naive_stats, true)
            .into_iter()
            .collect();
    assert_eq!(
        semi,
        naive,
        "semi-naive/naive closure divergence ({} facts) program:\n{src}",
        facts.len()
    );
    let reference = oracle::eval_naive(&mut d, &facts, &program, &strat);
    assert_eq!(
        semi,
        reference,
        "engine/oracle closure divergence ({} facts) program:\n{src}",
        facts.len()
    );
}

/// The sq-8sve7 soundness invariant, as a battery: recursive, NAF, aggregate and
/// multi-stratum programs × {empty graph, single fact, seeded random graphs} —
/// semi-naive output == naive output as SET equality on every input.
///
/// NON-VACUITY (verified by hand-run mutation, see the PR body): restricting the
/// semi-naive loop to `k = 0` only — or starting the delta empty instead of
/// all-facts — flips this test red on the recursive programs.
#[test]
fn differential_semi_naive_equals_naive_battery() {
    let programs = [
        // Pure positive recursion (transitive closure).
        format!(
            "{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n\
             [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:edge, ?z] ."
        ),
        // Transitive closure with the RECURSIVE atom in the SECOND body position —
        // the delta restriction must cover every positive-atom index k, not just
        // k = 0 (a k=0-only mutation goes red exactly here).
        format!(
            "{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n\
             [?x, ex:reach, ?z] :- [?y, ex:edge, ?z], [?x, ex:reach, ?y] ."
        ),
        // Recursion + NAF across strata (needs_full path + delta path together).
        format!(
            "{P}[?x, ex:reach, \"y\"] :- [?x, ex:seed, \"y\"] .\n\
             [?y, ex:reach, \"y\"] :- [?x, ex:reach, \"y\"], [?x, ex:edge, ?y] .\n\
             [?x, ex:unreach, \"y\"] :- [?x, a, ex:Node], NOT [?x, ex:reach, \"y\"] ."
        ),
        // Aggregate + threshold FILTER + NAF over a derived class (3 strata).
        format!(
            "{P}[?x, ex:deg, ?c] :- AGGREGATE([?x, ex:edge, ?y] ON ?x BIND COUNT(?y) AS ?c) .\n\
             [?x, a, ex:Hub] :- [?x, ex:deg, ?c], FILTER(?c >= 2) .\n\
             [?x, a, ex:Leaf] :- [?x, a, ex:Node], NOT [?x, a, ex:Hub] ."
        ),
        // Multi-head + recursion over a derived predicate + global count over the
        // recursive closure + a wildcard-NAF constant rule.
        format!(
            "{P}[?x, ex:out, ?y], [?y, ex:in, ?x] :- [?x, ex:edge, ?y] .\n\
             [?x, ex:reach, ?y] :- [?x, ex:out, ?y] .\n\
             [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:out, ?z] .\n\
             [ex:world, ex:nreach, ?c] :- AGGREGATE([?x, ex:reach, ?y] BIND COUNT(?x) AS ?c) .\n\
             [ex:world, ex:quiet, \"y\"] :- [ex:world, a, ex:Nothing], NOT [?z, ex:edge, ?z] ."
        ),
    ];
    for src in &programs {
        // Empty input graph.
        assert_semi_naive_equals_naive(src, &|_| Vec::new());
        // Single fact.
        assert_semi_naive_equals_naive(src, &|d| {
            let (a, e, b) = (iri(d, "a"), iri(d, "edge"), iri(d, "b"));
            vec![[a, e, b]]
        });
        // Seed-randomised graphs (typed nodes + random edges + one seed).
        for seed in 0..10 {
            assert_semi_naive_equals_naive(src, &move |d| random_graph(d, seed, 6, 9));
        }
    }
}

/// sq-8sve7 acceptance: on a 30-node linear-chain transitive closure, semi-naive
/// delta restriction feeds STRICTLY fewer candidate tuples into the join steps
/// than the forced-full (naive) discipline — while deriving the identical set,
/// cross-checked against the independent oracle. Deterministic counters only.
#[test]
fn semi_naive_considers_fewer_tuples_than_naive() {
    let mut d = Dict::new();
    let src = format!(
        "{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n\
         [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:edge, ?z] ."
    );
    let program = parse_program(&mut d, &src).expect("parse");
    let strat = stratify(&d, &program).expect("stratifiable");
    let edge = iri(&mut d, "edge");
    let nodes: Vec<Id> = (0..30).map(|i| iri(&mut d, &format!("c{i}"))).collect();
    let facts: Vec<[Id; 3]> = nodes.windows(2).map(|w| [w[0], edge, w[1]]).collect();

    let mut semi_stats = eval::EvalStats::default();
    let semi: FxHashSet<[Id; 3]> =
        eval::eval_stratified_with_stats(&mut d, &facts, &program, &strat, &mut semi_stats, false)
            .into_iter()
            .collect();
    let mut naive_stats = eval::EvalStats::default();
    let naive: FxHashSet<[Id; 3]> =
        eval::eval_stratified_with_stats(&mut d, &facts, &program, &strat, &mut naive_stats, true)
            .into_iter()
            .collect();

    // Identical derived sets, and both equal the independent oracle.
    let reference = oracle::eval_naive(&mut d, &facts, &program, &strat);
    assert_eq!(semi, reference);
    assert_eq!(naive, reference);
    // 30-node chain: 29 input edges + C(30,2) = 435 reach facts.
    assert_eq!(semi.len(), 29 + 435);

    // The point of the phase: strictly less join work, same fixpoint depth order.
    assert!(
        semi_stats.tuples_considered < naive_stats.tuples_considered,
        "semi-naive must consider strictly fewer tuples: semi={} naive={}",
        semi_stats.tuples_considered,
        naive_stats.tuples_considered
    );
    assert!(semi_stats.rounds >= 2, "chain TC needs multiple rounds");
    assert!(semi_stats.tuples_considered > 0, "counter must be live");
}

// ---------------------------------------------------------------------------
// Incremental maintenance (sq-4foq0): DRed for positive strata + rederivation
// at stratum boundaries, differentially pinned against from-scratch eval
// ---------------------------------------------------------------------------

use super::incr::UpdateStats;
use super::MaterializedProgram;

/// Closure of a materialization, as a set (with `len` cross-checked).
fn mat_set(m: &MaterializedProgram) -> FxHashSet<[Id; 3]> {
    let c = m.closure();
    assert_eq!(c.len(), m.len(), "closure() must be duplicate-free");
    c.into_iter().collect()
}

/// From-scratch reference closure over `facts`.
fn scratch_set(d: &mut Dict, facts: &[[Id; 3]], src: &str) -> FxHashSet<[Id; 3]> {
    let p = parse_program(d, src).expect("parse");
    eval(d, facts, &p)
        .expect("stratifiable")
        .into_iter()
        .collect()
}

/// Hand fixture: NAF flips BOTH WAYS under insert/delete (the non-monotonic
/// stratum re-derives at the boundary). NON-VACUITY (hand-run mutation, see the
/// PR body): treating NAF strata as positive (DRed) leaves the stale `orphan`
/// fact in place after the insert — this test goes red.
#[test]
fn incr_naf_flips_under_insert_and_delete() {
    let mut d = Dict::new();
    let src = format!("{P}[?x, ex:orphan, \"y\"] :- [?x, a, ex:Node], NOT [?p, ex:child, ?x] .");
    let (ty, node, child, orphan, y) = (
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        iri(&mut d, "Node"),
        iri(&mut d, "child"),
        iri(&mut d, "orphan"),
        s(&mut d, "y"),
    );
    let (a, p) = (iri(&mut d, "a"), iri(&mut d, "p"));
    let program = parse_program(&mut d, &src).unwrap();
    let mut m = MaterializedProgram::new(&mut d, &[[a, ty, node]], program).unwrap();
    assert!(m.contains(&[a, orphan, y]), "a starts orphaned");
    // Insert a child edge: the NAF derivation must RETRACT.
    m.insert(&mut d, &[[p, child, a]]);
    assert!(
        !m.contains(&[a, orphan, y]),
        "insert must retract the NAF fact"
    );
    assert_eq!(
        mat_set(&m),
        scratch_set(&mut d, &[[a, ty, node], [p, child, a]], &src)
    );
    // Delete it again: the NAF derivation must COME BACK.
    m.delete(&mut d, &[[p, child, a]]);
    assert!(
        m.contains(&[a, orphan, y]),
        "delete must restore the NAF fact"
    );
    assert_eq!(mat_set(&m), scratch_set(&mut d, &[[a, ty, node]], &src));
}

/// Hand fixture: the DIAMOND — a→b→d and a→c→d. Deleting edge b→d overdeletes
/// reach(a,d) (it was derived through b), and the rederivation pass must
/// REINSTATE it via the surviving c-path, while reach(b,d) stays gone.
/// NON-VACUITY (hand-run mutation, see the PR body): skipping the rederivation
/// pass (`over`-filter full pass) loses reach(a,d) — this test goes red.
#[test]
fn incr_diamond_delete_rederives_alternative_path() {
    let mut d = Dict::new();
    let src = format!(
        "{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n\
         [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:edge, ?z] ."
    );
    let (edge, reach) = (iri(&mut d, "edge"), iri(&mut d, "reach"));
    let (a, b, c, dd) = (
        iri(&mut d, "a"),
        iri(&mut d, "b"),
        iri(&mut d, "c"),
        iri(&mut d, "d"),
    );
    let facts = vec![[a, edge, b], [a, edge, c], [b, edge, dd], [c, edge, dd]];
    let program = parse_program(&mut d, &src).unwrap();
    let mut m = MaterializedProgram::new(&mut d, &facts, program).unwrap();
    assert!(m.contains(&[a, reach, dd]));
    let mut stats = UpdateStats::default();
    m.update_with_stats(&mut d, &[], &[[b, edge, dd]], &mut stats);
    assert!(
        m.contains(&[a, reach, dd]),
        "a→c→d survives: rederivation must reinstate"
    );
    assert!(!m.contains(&[b, reach, dd]), "b's only path is gone");
    assert!(!m.contains(&[b, edge, dd]));
    assert_eq!(
        stats.dred_strata, 1,
        "single positive stratum maintained by DRed"
    );
    assert_eq!(stats.recomputed_strata, 0);
    assert!(stats.overdeleted >= 2, "b→d and a→d (at least) overdelete");
    assert!(stats.rederived >= 1, "a→d must be rederived");
    let remaining = vec![[a, edge, b], [a, edge, c], [c, edge, dd]];
    assert_eq!(mat_set(&m), scratch_set(&mut d, &remaining, &src));
}

/// Hand fixture: deleting a chain's first edge retracts the whole dependent
/// cone (transitive overdeletion), and nothing else.
#[test]
fn incr_chain_delete_retracts_dependent_cone() {
    let mut d = Dict::new();
    let src = format!(
        "{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n\
         [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:edge, ?z] ."
    );
    let (edge, reach) = (iri(&mut d, "edge"), iri(&mut d, "reach"));
    let n: Vec<Id> = (0..4).map(|i| iri(&mut d, &format!("n{i}"))).collect();
    let facts: Vec<[Id; 3]> = n.windows(2).map(|w| [w[0], edge, w[1]]).collect();
    let program = parse_program(&mut d, &src).unwrap();
    let mut m = MaterializedProgram::new(&mut d, &facts, program).unwrap();
    let lost = m.delete(&mut d, &[[n[0], edge, n[1]]]);
    // Lost: the edge itself + reach(0,1), reach(0,2), reach(0,3).
    assert_eq!(lost, 4);
    for j in 1..4 {
        assert!(!m.contains(&[n[0], reach, n[j]]));
    }
    assert!(
        m.contains(&[n[1], reach, n[3]]),
        "the 1→2→3 cone is untouched"
    );
    assert_eq!(mat_set(&m), scratch_set(&mut d, &facts[1..], &src));
}

/// A deleted BASE fact that is still derivable must STAY in the closure (it
/// changes owner: base → derived), and deleting a derived-only fact is a no-op.
#[test]
fn incr_delete_of_still_derivable_fact_keeps_it() {
    let mut d = Dict::new();
    let src = format!("{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .");
    let (edge, reach) = (iri(&mut d, "edge"), iri(&mut d, "reach"));
    let (a, b) = (iri(&mut d, "a"), iri(&mut d, "b"));
    // reach(a,b) is BOTH asserted and derivable from edge(a,b).
    let program = parse_program(&mut d, &src).unwrap();
    let mut m = MaterializedProgram::new(&mut d, &[[a, edge, b], [a, reach, b]], program).unwrap();
    assert_eq!(m.len(), 2);
    // Deleting the asserted copy: still derivable → closure unchanged.
    assert_eq!(m.delete(&mut d, &[[a, reach, b]]), 0);
    assert!(m.contains(&[a, reach, b]));
    // Deleting a fact that is only DERIVED (no longer asserted): no-op.
    assert_eq!(m.delete(&mut d, &[[a, reach, b]]), 0);
    assert!(m.contains(&[a, reach, b]));
    // Removing the support finally retracts it.
    assert_eq!(m.delete(&mut d, &[[a, edge, b]]), 2);
    assert!(m.is_empty());
}

/// Aggregate threshold crossing in BOTH directions across three strata
/// (COUNT → FILTER → NAF): inserts create Hub and retract Leaf; deletes undo.
#[test]
fn incr_aggregate_threshold_crosses_both_ways() {
    let mut d = Dict::new();
    let src = format!(
        "{P}[?x, ex:deg, ?c] :- AGGREGATE([?x, ex:edge, ?y] ON ?x BIND COUNT(?y) AS ?c) .\n\
         [?x, a, ex:Hub] :- [?x, ex:deg, ?c], FILTER(?c >= 2) .\n\
         [?x, a, ex:Leaf] :- [?x, a, ex:Node], NOT [?x, a, ex:Hub] ."
    );
    let (ty, node, hub, leaf, edge) = (
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        iri(&mut d, "Node"),
        iri(&mut d, "Hub"),
        iri(&mut d, "Leaf"),
        iri(&mut d, "edge"),
    );
    let (a, b, c) = (iri(&mut d, "a"), iri(&mut d, "b"), iri(&mut d, "c"));
    let mut facts = vec![[a, ty, node], [a, edge, b]];
    let program = parse_program(&mut d, &src).unwrap();
    let mut m = MaterializedProgram::new(&mut d, &facts, program).unwrap();
    assert!(m.contains(&[a, ty, leaf]) && !m.contains(&[a, ty, hub]));
    // Second edge: count crosses the threshold — Hub appears, Leaf retracts.
    m.insert(&mut d, &[[a, edge, c]]);
    facts.push([a, edge, c]);
    assert!(m.contains(&[a, ty, hub]) && !m.contains(&[a, ty, leaf]));
    assert_eq!(mat_set(&m), scratch_set(&mut d, &facts, &src));
    // Delete one edge: back under the threshold — Hub retracts, Leaf returns.
    m.delete(&mut d, &[[a, edge, b]]);
    facts.retain(|f| *f != [a, edge, b]);
    assert!(!m.contains(&[a, ty, hub]) && m.contains(&[a, ty, leaf]));
    assert_eq!(mat_set(&m), scratch_set(&mut d, &facts, &src));
}

/// Batched update semantics: `new base = (base \ deletes) ∪ inserts`, a fact in
/// both survives, and the returned pair counts the exact closure diff.
#[test]
fn incr_batched_update_semantics_and_counts() {
    let mut d = Dict::new();
    let src = format!("{P}[?x, ex:q, ?y] :- [?x, ex:p, ?y] .");
    let (p, q) = (iri(&mut d, "p"), iri(&mut d, "q"));
    let (a, b, c) = (iri(&mut d, "a"), iri(&mut d, "b"), iri(&mut d, "c"));
    let program = parse_program(&mut d, &src).unwrap();
    let mut m = MaterializedProgram::new(&mut d, &[[a, p, b]], program).unwrap();
    // Replace a→b with a→c in one batch: gain {apc,aqc}, lose {apb,aqb}.
    assert_eq!(m.update(&mut d, &[[a, p, c]], &[[a, p, b]]), (2, 2));
    assert!(m.contains(&[a, q, c]) && !m.contains(&[a, q, b]));
    // A fact in BOTH inserts and deletes survives (single-batch semantics).
    assert_eq!(m.update(&mut d, &[[a, p, c]], &[[a, p, c]]), (0, 0));
    assert!(m.contains(&[a, q, c]));
    // No-op update returns (0, 0) without touching anything.
    assert_eq!(m.update(&mut d, &[], &[]), (0, 0));
    assert_eq!(m.len(), 2);
}

/// Strata whose rules read NONE of the changed predicates are skipped outright
/// (no recompute, no DRed) — including non-monotonic strata, which would
/// otherwise pay a full stratum rederivation for an irrelevant change.
#[test]
fn incr_skips_unaffected_strata() {
    let mut d = Dict::new();
    // Stratum 0 owns p→q; the NAF stratum reads only rdf:type/r.
    let src = format!(
        "{P}[?x, ex:q, ?y] :- [?x, ex:p, ?y] .\n\
         [?x, ex:t, \"y\"] :- [?x, a, ex:Node], NOT [?z, ex:r, ?x] ."
    );
    let (ty, node, p) = (
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        iri(&mut d, "Node"),
        iri(&mut d, "p"),
    );
    let (a, b) = (iri(&mut d, "a"), iri(&mut d, "b"));
    let program = parse_program(&mut d, &src).unwrap();
    let mut m = MaterializedProgram::new(&mut d, &[[a, ty, node]], program).unwrap();
    // Insert a p-fact: only the positive stratum is affected; the NAF stratum
    // (whose input predicates did not change) must be SKIPPED, not recomputed.
    let mut stats = UpdateStats::default();
    m.update_with_stats(&mut d, &[[a, p, b]], &[], &mut stats);
    assert_eq!(stats.recomputed_strata, 0, "NAF stratum must not recompute");
    assert!(stats.skipped_strata >= 1);
    assert_eq!(
        mat_set(&m),
        scratch_set(&mut d, &[[a, ty, node], [a, p, b]], &src)
    );
    // And deleting it again is equally skip-clean for the NAF stratum.
    let mut stats = UpdateStats::default();
    m.update_with_stats(&mut d, &[], &[[a, p, b]], &mut stats);
    assert_eq!(stats.recomputed_strata, 0);
    assert_eq!(mat_set(&m), scratch_set(&mut d, &[[a, ty, node]], &src));
}

/// The point of the phase, deterministically: on a 30-node chain transitive
/// closure, maintaining ONE deleted tail edge (DRed) and ONE inserted extension
/// edge each feed strictly fewer candidate tuples into the join steps than a
/// from-scratch re-evaluation of the same new base. NO wall-clock.
#[test]
fn incr_maintenance_considers_fewer_tuples_than_from_scratch() {
    let mut d = Dict::new();
    let src = format!(
        "{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n\
         [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:edge, ?z] ."
    );
    let edge = iri(&mut d, "edge");
    let nodes: Vec<Id> = (0..31).map(|i| iri(&mut d, &format!("c{i}"))).collect();
    let facts: Vec<[Id; 3]> = nodes[..30].windows(2).map(|w| [w[0], edge, w[1]]).collect();
    let program = parse_program(&mut d, &src).unwrap();
    let mut m = MaterializedProgram::new(&mut d, &facts, program.clone()).unwrap();

    // DELETE the tail edge c28→c29.
    let mut del_stats = UpdateStats::default();
    m.update_with_stats(&mut d, &[], &[[nodes[28], edge, nodes[29]]], &mut del_stats);
    let strat = stratify(&d, &program).unwrap();
    let mut scratch_stats = eval::EvalStats::default();
    let scratch: FxHashSet<[Id; 3]> = eval::eval_stratified_with_stats(
        &mut d,
        &facts[..28],
        &program,
        &strat,
        &mut scratch_stats,
        false,
    )
    .into_iter()
    .collect();
    assert_eq!(mat_set(&m), scratch);
    assert!(
        del_stats.eval.tuples_considered < scratch_stats.tuples_considered,
        "DRed delete must beat from-scratch: incr={} scratch={}",
        del_stats.eval.tuples_considered,
        scratch_stats.tuples_considered
    );

    // INSERT an extension edge c29→c30 (pure-insert path: no rederivation pass).
    let mut ins_stats = UpdateStats::default();
    m.update_with_stats(&mut d, &[[nodes[29], edge, nodes[30]]], &[], &mut ins_stats);
    let new_base: Vec<[Id; 3]> = facts[..28]
        .iter()
        .copied()
        .chain([[nodes[29], edge, nodes[30]]])
        .collect();
    let mut scratch_stats2 = eval::EvalStats::default();
    let scratch2: FxHashSet<[Id; 3]> = eval::eval_stratified_with_stats(
        &mut d,
        &new_base,
        &program,
        &strat,
        &mut scratch_stats2,
        false,
    )
    .into_iter()
    .collect();
    assert_eq!(mat_set(&m), scratch2);
    assert!(
        ins_stats.eval.tuples_considered < scratch_stats2.tuples_considered,
        "insert must beat from-scratch: incr={} scratch={}",
        ins_stats.eval.tuples_considered,
        scratch_stats2.tuples_considered
    );
    assert!(ins_stats.eval.tuples_considered > 0, "counter must be live");
}

/// The Phase-3 soundness invariant, as a battery: after EVERY step of a seeded
/// random insert/delete sequence, the incrementally maintained closure equals a
/// from-scratch [`eval`] over the current base — across recursive, NAF,
/// aggregate and multi-stratum programs.
///
/// NON-VACUITY (verified by hand-run mutation, see the PR body): (a) seeding
/// DRed overdeletion with the removals but skipping its propagation loop,
/// (b) dropping the rederivation pass, and (c) treating every stratum as
/// skippable each flip this battery red.
#[test]
fn incr_differential_random_insert_delete_sequences() {
    let programs = [
        // Pure positive recursion (transitive closure) — the DRed path.
        format!(
            "{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n\
             [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:edge, ?z] ."
        ),
        // Recursion + NAF across strata (DRed below, boundary rederivation above).
        format!(
            "{P}[?x, ex:reach, \"y\"] :- [?x, ex:seed, \"y\"] .\n\
             [?y, ex:reach, \"y\"] :- [?x, ex:reach, \"y\"], [?x, ex:edge, ?y] .\n\
             [?x, ex:unreach, \"y\"] :- [?x, a, ex:Node], NOT [?x, ex:reach, \"y\"] ."
        ),
        // Aggregate + threshold FILTER + NAF over a derived class (3 strata).
        format!(
            "{P}[?x, ex:deg, ?c] :- AGGREGATE([?x, ex:edge, ?y] ON ?x BIND COUNT(?y) AS ?c) .\n\
             [?x, a, ex:Hub] :- [?x, ex:deg, ?c], FILTER(?c >= 2) .\n\
             [?x, a, ex:Leaf] :- [?x, a, ex:Node], NOT [?x, a, ex:Hub] ."
        ),
        // Numeric aggregates over mutating weights (SUM/MIN/MAX/AVG minting).
        format!(
            "{P}[ex:world, ex:total, ?s] :- AGGREGATE([?x, ex:weight, ?v] BIND SUM(?v) AS ?s) .\n\
             [ex:world, ex:min, ?v] :- AGGREGATE([?x, ex:weight, ?n] BIND MIN(?n) AS ?v) .\n\
             [ex:world, ex:avg, ?v] :- AGGREGATE([?x, ex:weight, ?n] BIND AVG(?n) AS ?v) .\n\
             [ex:world, ex:large, \"y\"] :- [ex:world, ex:total, ?s], FILTER(?s > 10) ."
        ),
        // Multi-head + recursion over a derived predicate + global count.
        format!(
            "{P}[?x, ex:out, ?y], [?y, ex:in, ?x] :- [?x, ex:edge, ?y] .\n\
             [?x, ex:reach, ?y] :- [?x, ex:out, ?y] .\n\
             [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:out, ?z] .\n\
             [ex:world, ex:nreach, ?c] :- AGGREGATE([?x, ex:reach, ?y] BIND COUNT(?x) AS ?c) ."
        ),
    ];
    for src in &programs {
        for seed in 0..6u64 {
            let mut d = Dict::new();
            let program = parse_program(&mut d, src).expect("parse");
            let edge = iri(&mut d, "edge");
            let weight = iri(&mut d, "weight");
            let nodes: Vec<Id> = (0..6).map(|i| iri(&mut d, &format!("n{i}"))).collect();
            // Deduplicate: the mirror below tracks BASE-set membership, and
            // `random_graph` may draw the same edge twice.
            let mut seen: FxHashSet<[Id; 3]> = FxHashSet::default();
            let mut facts: Vec<[Id; 3]> = random_graph(&mut d, seed, 6, 9)
                .into_iter()
                .filter(|f| seen.insert(*f))
                .collect();
            let mut m =
                MaterializedProgram::new(&mut d, &facts, program.clone()).expect("stratifiable");
            let mut rng = Lcg(seed.wrapping_mul(0x517cc1b727220a95).wrapping_add(3));
            for step in 0..12 {
                match rng.next(3) {
                    0 if !facts.is_empty() => {
                        // Delete a random CURRENT base fact (any kind: edge,
                        // type, weight, seed — exercises NAF/aggregate flips).
                        let i = rng.next(facts.len() as u64) as usize;
                        let f = facts.swap_remove(i);
                        m.delete(&mut d, &[f]);
                    }
                    1 => {
                        let f = [
                            nodes[rng.next(6) as usize],
                            edge,
                            nodes[rng.next(6) as usize],
                        ];
                        if !facts.contains(&f) {
                            facts.push(f);
                        }
                        m.insert(&mut d, &[f]);
                    }
                    _ => {
                        let v = int(&mut d, rng.next(9) + 1);
                        let f = [nodes[rng.next(6) as usize], weight, v];
                        if !facts.contains(&f) {
                            facts.push(f);
                        }
                        m.insert(&mut d, &[f]);
                    }
                }
                let reference: FxHashSet<[Id; 3]> = eval(&mut d, &facts, &program)
                    .expect("stratifiable")
                    .into_iter()
                    .collect();
                assert_eq!(
                    mat_set(&m),
                    reference,
                    "incremental/from-scratch divergence at step {step} seed {seed} program:\n{src}"
                );
            }
        }
    }
}

/// Non-stratifiable programs are rejected at construction (via `stratify`).
#[test]
fn incr_new_rejects_non_stratifiable() {
    let mut d = Dict::new();
    let p = parse_program(
        &mut d,
        &format!("{P}[?x, ex:win, \"y\"] :- [?x, ex:move, ?y], NOT [?y, ex:win, \"y\"] ."),
    )
    .unwrap();
    assert!(MaterializedProgram::new(&mut d, &[], p).is_err());
}

/// Direct exercises of the small accessors (coverage floor: every new public
/// fn has a direct unit test): duplicate inserts, `contains` on all layers,
/// `closure` set-ness, `len`/`is_empty`.
#[test]
fn incr_accessors_direct() {
    let mut d = Dict::new();
    let src = format!("{P}[?x, ex:q, ?y] :- [?x, ex:p, ?y] .");
    let (p, q) = (iri(&mut d, "p"), iri(&mut d, "q"));
    let (a, b) = (iri(&mut d, "a"), iri(&mut d, "b"));
    let program = parse_program(&mut d, &src).unwrap();
    let mut m = MaterializedProgram::new(&mut d, &[], program).unwrap();
    assert!(m.is_empty() && m.closure().is_empty());
    assert_eq!(m.len(), 0);
    assert_eq!(m.insert(&mut d, &[[a, p, b], [a, p, b]]), 2); // dup input, one fact
    assert_eq!(m.insert(&mut d, &[[a, p, b]]), 0); // already asserted
    assert_eq!(m.insert(&mut d, &[[a, q, b]]), 0); // already DERIVED: closure unchanged
    assert!(m.contains(&[a, p, b]) && m.contains(&[a, q, b]));
    assert!(!m.contains(&[b, p, a]));
    assert!(!m.is_empty());
    assert_eq!(m.len(), 2);
    // The derived fact was ALSO asserted above; retracting its support must
    // keep it (it is still asserted), then deleting the assertion clears it.
    assert_eq!(m.delete(&mut d, &[[a, p, b]]), 1);
    assert!(m.contains(&[a, q, b]) && !m.contains(&[a, p, b]));
    assert_eq!(m.delete(&mut d, &[[a, q, b]]), 1);
    assert!(m.is_empty());
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] sq-xzb9p — differential vs the EXTERNAL Soufflé engine
// ---------------------------------------------------------------------------
// `oracle` is an independent implementation, but it is still OUR reading of the
// semantics. These fixtures put the same programs through Soufflé — which stratifies
// them ITSELF, so this arm can catch a stratification bug the in-tree oracle cannot.
// The translation is pinned by a golden test that runs with or without the binary;
// the engine comparison skips (loudly, "not checked") when Soufflé is absent. See
// `super::souffle` for the fragment this arm covers and why it stops where it does.

/// The Soufflé closure and ours must agree on every triple the program's relations
/// cover, over the same seed-randomised graphs the oracle arm uses.
fn assert_souffle_agrees(src: &str, seeds: std::ops::Range<u64>) {
    let Some(bin) = souffle::binary() else {
        return;
    };
    for seed in seeds {
        let mut d = Dict::new();
        let program = parse_program(&mut d, src).expect("parse");
        let facts = random_graph(&mut d, seed, 6, 9);
        let translation = souffle::translate(&d, &program, &facts).expect("in fragment");
        let external = translation.run(&bin).expect("souffle run");
        let ours: FxHashSet<[Id; 3]> = eval(&mut d, &facts, &program)
            .unwrap()
            .into_iter()
            .filter(|&t| translation.covers(t))
            .collect();
        assert_eq!(
            ours, external,
            "sparq/souffle divergence: seed {seed} program:\n{src}"
        );
    }
}

#[test]
fn souffle_recursion_plus_naf() {
    assert_souffle_agrees(
        &format!(
            "{P}[?x, ex:reach, \"y\"] :- [?x, ex:seed, \"y\"] .\n\
             [?y, ex:reach, \"y\"] :- [?x, ex:reach, \"y\"], [?x, ex:edge, ?y] .\n\
             [?x, ex:unreach, \"y\"] :- [?x, a, ex:Node], NOT [?x, ex:reach, \"y\"] ."
        ),
        0..15,
    );
}

/// The grouped `NOT` is one existential conjunction; the aux-relation encoding has to
/// preserve that, so an external engine disagreeing here would mean our join-inside-the-
/// group semantics is wrong.
#[test]
fn souffle_naf_conjunction() {
    assert_souffle_agrees(
        &format!(
            "{P}[?x, ex:noCycle, \"y\"] :- [?x, a, ex:Node], \
             NOT {{ [?x, ex:edge, ?y], [?y, ex:edge, ?x] }} ."
        ),
        0..15,
    );
}

/// Transitive closure, a multi-atom head, and an UNCORRELATED wildcard `NOT` with a
/// repeated variable (`[?z, ex:edge, ?z]`) — which translates to a NULLARY auxiliary
/// relation, the encoding's most easily-broken corner.
///
/// The guarded rule must actually be EXERCISED both ways or this fixture would pass
/// with the negation dropped entirely (an earlier draft used a guard atom the random
/// graph never produces, and survived exactly that mutation). So the seed range is
/// required to contain a graph with a self-loop and one without, and the derivation is
/// asserted to track it.
#[test]
fn souffle_multi_head_and_wildcard_naf() {
    let src = format!(
        "{P}[?x, ex:reach, ?y] :- [?x, ex:edge, ?y] .\n\
         [?x, ex:reach, ?z] :- [?x, ex:reach, ?y], [?y, ex:edge, ?z] .\n\
         [?x, ex:out, ?y], [?y, ex:in, ?x] :- [?x, ex:edge, ?y] .\n\
         [?x, ex:loopFree, \"y\"] :- [?x, a, ex:Node], NOT [?z, ex:edge, ?z] ."
    );
    assert_souffle_agrees(&src, 0..15);
    // Witness that the nullary NOT gates the rule in BOTH directions across the range.
    let (mut fired, mut blocked) = (0, 0);
    for seed in 0..15 {
        let mut d = Dict::new();
        let program = parse_program(&mut d, &src).unwrap();
        let facts = random_graph(&mut d, seed, 6, 9);
        let edge = iri(&mut d, "edge");
        let loop_free = iri(&mut d, "loopFree");
        let self_loop = facts.iter().any(|&[s, p, o]| p == edge && s == o);
        let derived_any = eval(&mut d, &facts, &program)
            .unwrap()
            .iter()
            .any(|&[_, p, _]| p == loop_free);
        assert_eq!(
            derived_any, !self_loop,
            "seed {seed}: ex:loopFree must be derived exactly when no self-loop exists"
        );
        if derived_any {
            fired += 1;
        } else {
            blocked += 1;
        }
    }
    assert!(
        fired > 0 && blocked > 0,
        "the seed range must exercise the nullary NOT both ways (fired {fired}, blocked {blocked})"
    );
}

/// The class-granularity decision, externally corroborated: `NOT [?x, a, ex:Hub]`
/// feeding an `ex:Leaf` head is stratifiable ONLY if `rdf:type` nodes are per-CLASS.
/// A predicate-granular encoding would make Soufflé reject this program outright, so
/// Soufflé accepting it is independent evidence for the design record's §3 choice.
#[test]
fn souffle_accepts_class_granular_negation() {
    assert_souffle_agrees(
        &format!(
            "{P}[?x, a, ex:Hub] :- [?x, ex:edge, ?y] .\n\
             [?x, a, ex:Leaf] :- [?x, a, ex:Node], NOT [?x, a, ex:Hub] ."
        ),
        0..15,
    );
}

/// Rejection parity: the textbook `win(X) :- move(X,Y), NOT win(Y)` has no stratified
/// model, and BOTH engines must say so. Agreeing only on accepted programs would let a
/// too-permissive checker pass unnoticed.
#[test]
fn souffle_rejects_what_our_checker_rejects() {
    let src = format!(
        "{P}[?x, ex:win, \"y\"] :- [?x, ex:move, ?y], NOT [?y, ex:win, \"y\"] ."
    );
    let mut d = Dict::new();
    let program = parse_program(&mut d, &src).expect("parse");
    assert!(
        stratify(&d, &program).is_err(),
        "our checker must reject the negation cycle"
    );
    let Some(bin) = souffle::binary() else {
        return;
    };
    let facts = random_graph(&mut d, 0, 6, 9);
    let translation = souffle::translate(&d, &program, &facts).expect("in fragment");
    let err = translation
        .run(&bin)
        .expect_err("souffle must also refuse to stratify this program");
    assert!(
        err.contains("stratify"),
        "souffle refused for an unexpected reason: {err}"
    );
}

/// The fragment boundary is a LOUD error naming the construct, never a silent partial
/// translation that would quietly compare fewer rules than the program has.
#[test]
fn souffle_translation_rejects_out_of_fragment_constructs() {
    let cases = [
        (
            format!("{P}[?x, ex:deg, ?c] :- AGGREGATE([?x, ex:edge, ?y] ON ?x BIND COUNT(?y) AS ?c) ."),
            "AGGREGATE",
        ),
        (
            format!("{P}[?x, ex:big, \"y\"] :- [?x, ex:deg, ?c], FILTER(?c >= 2) ."),
            "FILTER",
        ),
        (
            format!("{P}[?p, ex:observed, ?y] :- [?x, ?p, ?y] ."),
            "variable predicates",
        ),
        (
            format!("{P}[?x, ex:typed, \"y\"] :- [?x, a, ?c] ."),
            "variable-class",
        ),
    ];
    for (src, needle) in cases {
        let mut d = Dict::new();
        let program = parse_program(&mut d, &src).expect("parse");
        let err = souffle::translate(&d, &program, &[]).expect_err("out of fragment");
        assert!(
            err.contains(needle),
            "expected the error to name {needle:?}, got: {err}"
        );
    }
}

/// The run path up to the process spawn — scratch directory, program file, and one
/// `.facts` table per relation — is exercised even on a box with no Soufflé, so an
/// ordinary CI run still covers it and a spawn failure is reported, never swallowed.
#[test]
fn souffle_run_reports_an_unrunnable_binary() {
    let mut d = Dict::new();
    let src = format!("{P}[?x, ex:q, ?y] :- [?x, ex:p, ?y] .");
    let program = parse_program(&mut d, &src).unwrap();
    let (a, p, b) = (iri(&mut d, "a"), iri(&mut d, "p"), iri(&mut d, "b"));
    let t = souffle::translate(&d, &program, &[[a, p, b]]).unwrap();
    let err = t
        .run("sparq-no-such-souffle-binary")
        .expect_err("an absent binary must be an error, not an empty closure");
    assert!(
        err.contains("failed to run"),
        "expected a spawn-failure report, got: {err}"
    );
}

/// Golden translation: the emitted Soufflé source is the artifact a reviewer reads and
/// the CI lane runs, so it is pinned exactly. This runs whether or not Soufflé is
/// installed, which is what keeps the translator covered on an ordinary CI box.
#[test]
fn souffle_translation_is_pinned() {
    let mut d = Dict::new();
    let src = format!(
        "{P}[?y, ex:reach, \"y\"] :- [?x, ex:reach, \"y\"], [?x, ex:edge, ?y] .\n\
         [?x, ex:lonely, \"y\"] :- [?x, a, ex:Node], NOT {{ [?x, ex:edge, ?w] }} ."
    );
    let program = parse_program(&mut d, &src).unwrap();
    let (a, b) = (iri(&mut d, "a"), iri(&mut d, "b"));
    let (edge, ty, node) = (
        iri(&mut d, "edge"),
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        iri(&mut d, "Node"),
    );
    let t = souffle::translate(&d, &program, &[[a, edge, b], [a, ty, node]]).unwrap();
    assert_eq!(
        t.source,
        "// Generated by sparq-reason datalog::souffle (sq-xzb9p). Do not edit.\n\
         // One relation per stratification node; Soufflé stratifies this itself.\n\
         .decl p0_reach(s: symbol, o: symbol)\n\
         .input p0_reach\n\
         .decl p1_edge(s: symbol, o: symbol)\n\
         .input p1_edge\n\
         .decl c2_Node(s: symbol)\n\
         .input c2_Node\n\
         .decl p3_lonely(s: symbol, o: symbol)\n\
         .input p3_lonely\n\
         .decl neg1_0(v0: symbol)\n\
         p0_reach(v0, \"\\\"y\\\"\") :- p0_reach(v1, \"\\\"y\\\"\"), p1_edge(v1, v0).\n\
         neg1_0(v0) :- p1_edge(v0, v1).\n\
         p3_lonely(v0, \"\\\"y\\\"\") :- c2_Node(v0), !neg1_0(v0).\n\
         .output p0_reach\n\
         .output p1_edge\n\
         .output c2_Node\n\
         .output p3_lonely\n"
    );
    // A triple under a predicate no atom reads is outside the comparison projection.
    assert!(t.covers([a, edge, b]) && t.covers([a, ty, node]));
    assert!(!t.covers([a, iri(&mut d, "weight"), b]));
}
