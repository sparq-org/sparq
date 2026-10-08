//! `log:collectAllIn` / `log:forAllIn` (scoped aggregation) and the multi-stratum
//! entry point `reason_n3_stratified` — the small N3 follow-ups surfaced by the
//! Solid access-control pipeline (research/solid-access-control-design.md §1.4 /
//! §7 item 2, bead sq-jwsp).
//!
//! 🤖 SPARQ agent — sq-jwsp [FABLE-5].
//!
//! Semantics pinned here:
//!   * collectAllIn is findall — duplicates kept, no solutions ⇒ the empty list —
//!     and composes with `math:memberCount` for count-over-property-values (the
//!     shape that collapses the ACP allOf strata).
//!   * forAllIn is scoped universal quantification (vacuously true), the direct
//!     allOf collapse.
//!   * A bound `{ … }` scope is matched syntactically; an unbound scope variable
//!     is the current store (the engine's log:includes convention).
//!   * Malformed operands FAIL the premise (fail-closed), never mis-fire.
//!   * `reason_n3_stratified` carries each stratum's term-level closure into the
//!     next with per-stratum blank-node scope, making non-monotonic operators
//!     sound over earlier strata's derived predicates.

use sparq_core::dict::Dict;
use sparq_reason::{reason_n3, reason_n3_stratified};
use std::collections::HashSet;

/// Closure of `src` as canonical (s, p, o) strings in `Dict::term` rendering.
fn closure(src: &str) -> HashSet<(String, String, String)> {
    let mut d = Dict::new();
    reason_n3(&mut d, src)
        .expect("reasoning failed")
        .iter()
        .map(|[s, p, o]| (d.term(*s).to_string(), d.term(*p).to_string(), d.term(*o).to_string()))
        .collect()
}

/// Stratified closure as canonical (s, p, o) strings, plus the per-stratum sizes.
fn strat_closure(strata: &[&str]) -> (HashSet<(String, String, String)>, Vec<usize>) {
    let mut d = Dict::new();
    let out = reason_n3_stratified(&mut d, strata).expect("stratified reasoning failed");
    let set = out
        .facts
        .iter()
        .map(|[s, p, o]| (d.term(*s).to_string(), d.term(*p).to_string(), d.term(*o).to_string()))
        .collect();
    (set, out.strata_facts)
}

const PRE: &str = "@prefix : <http://ex/> .\n\
    @prefix math: <http://www.w3.org/2000/10/swap/math#> .\n\
    @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n";

fn run(body: &str) -> HashSet<(String, String, String)> {
    closure(&format!("{}{}", PRE, body))
}

fn iri(local: &str) -> String {
    format!("<http://ex/{}>", local)
}
fn int_lit(n: usize) -> String {
    format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>", n)
}
fn has(c: &HashSet<(String, String, String)>, s: &str, p: &str, o: String) -> bool {
    c.contains(&(iri(s), iri(p), o))
}

// ---------- log:collectAllIn ----------

#[test]
fn collect_all_counts_property_values_per_subject() {
    // The ACP shape: count the VALUES of a property, correlated per subject —
    // what math:memberCount (list/formula-only) could not express by itself.
    let c = run(":alice a :Group . :bob a :Group .\n\
         :alice :member :m1 . :alice :member :m2 . :alice :member :m3 .\n\
         :bob :member :m4 .\n\
         { ?g a :Group . ( ?m { ?g :member ?m } ?l ) log:collectAllIn ?scope .\n\
           ?l math:memberCount ?n . } => { ?g :size ?n } .");
    assert!(has(&c, "alice", "size", int_lit(3)), "alice has 3 members; got {:?}", c);
    assert!(has(&c, "bob", "size", int_lit(1)), "bob has 1 member; got {:?}", c);
}

#[test]
fn collect_all_no_solutions_binds_the_empty_list() {
    let c = run(":c a :Group .\n\
         { ?g a :Group . ( ?m { ?g :member ?m } ?l ) log:collectAllIn ?x .\n\
           ?l math:memberCount ?n . } => { ?g :size ?n } .");
    assert!(has(&c, "c", "size", int_lit(0)), "no members ⇒ () ⇒ count 0; got {:?}", c);
}

#[test]
fn collect_all_keeps_duplicates_findall_semantics() {
    // Template ?x has one instantiation PER SOLUTION of the clause: :s1 is
    // collected once per :p value (findall, not a distinct set).
    let c = run(":s1 :p :a . :s1 :p :b .\n\
         { ( ?x { ?x :p ?y } ?l ) log:collectAllIn ?w . ?l math:memberCount ?n . }\n\
           => { :r :count ?n } .");
    assert!(has(&c, "r", "count", int_lit(2)), "2 solutions ⇒ 2 members; got {:?}", c);
}

#[test]
fn collect_all_over_a_bound_formula_scope() {
    // A ground `{ … }` scope is matched syntactically (log:includes convention),
    // not against the store.
    let c = run(":ignored :store :fact .\n\
         { ( ?x { ?x a :P } ?l ) log:collectAllIn { :a a :P . :b a :P . :c a :Q } .\n\
           ?l math:memberCount ?n . } => { :r :count ?n } .");
    assert!(has(&c, "r", "count", int_lit(2)), "2 of 3 scope triples match; got {:?}", c);
}

#[test]
fn collect_all_malformed_operands_fail_closed() {
    // Wrong subject arity: 2-list instead of ( template clause list ).
    let c = run(":s :p :a .\n\
         { ( ?m { ?s :p ?m } ) log:collectAllIn ?x . } => { :r :fired true } .");
    assert!(!c.iter().any(|(s, _, _)| s == &iri("r")), "2-list subject must not fire; got {:?}", c);
    // Non-formula clause.
    let c2 = run(":s :p :a .\n\
         { ( ?m :notaformula ?l ) log:collectAllIn ?x . } => { :r :fired true } .");
    assert!(!c2.iter().any(|(s, _, _)| s == &iri("r")), "IRI clause must not fire; got {:?}", c2);
    // Non-list subject.
    let c3 = run(":s :p :a .\n\
         { :s log:collectAllIn ?x . } => { :r :fired true } .");
    assert!(!c3.iter().any(|(s, _, _)| s == &iri("r")), "non-list subject must not fire; got {:?}", c3);
}

// ---------- log:forAllIn ----------

#[test]
fn for_all_in_collapses_the_all_of_shape() {
    // allOf: a policy applies iff EVERY one of its matchers is satisfied.
    let c = run(":pol a :Policy . :pol2 a :Policy .\n\
         :pol :matcher :m1 . :pol :matcher :m2 .\n\
         :pol2 :matcher :m3 . :pol2 :matcher :m4 .\n\
         :m1 :satisfied true . :m2 :satisfied true . :m3 :satisfied true .\n\
         { ?p a :Policy . ( { ?p :matcher ?m } { ?m :satisfied true } ) log:forAllIn ?w . }\n\
           => { ?p :allOf true } .");
    let t = "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string();
    assert!(has(&c, "pol", "allOf", t.clone()), "all matchers satisfied; got {:?}", c);
    assert!(!has(&c, "pol2", "allOf", t), "m4 unsatisfied ⇒ no allOf; got {:?}", c);
}

#[test]
fn for_all_in_is_vacuously_true() {
    // No clause-a solutions ⇒ the universal holds (standard ∀ semantics; an
    // allOf policy with zero matchers applies — guard with a non-emptiness
    // atom if that is not wanted).
    let c = run(":pol a :Policy .\n\
         { ?p a :Policy . ( { ?p :matcher ?m } { ?m :satisfied true } ) log:forAllIn ?w . }\n\
           => { ?p :allOf true } .");
    let t = "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string();
    assert!(has(&c, "pol", "allOf", t), "vacuous ∀ holds; got {:?}", c);
}

#[test]
fn for_all_in_over_a_bound_formula_scope() {
    let c = run("{ ( { ?x a :P } { ?x :ok true } ) log:forAllIn\n\
             { :a a :P . :a :ok true . :b a :P . :b :ok true . :c a :Q } . }\n\
           => { :r :fired true } .");
    let t = "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string();
    assert!(has(&c, "r", "fired", t), "every :P in scope is :ok; got {:?}", c);
}

// ---------- reason_n3_stratified ----------

#[test]
fn stratified_carries_the_closure_between_strata() {
    let (c, sizes) = strat_closure(&[
        "@prefix : <http://ex/> .\n:a :p :b .\n{ ?x :p ?y } => { ?x :q ?y } .",
        "@prefix : <http://ex/> .\n{ ?x :q ?y } => { ?x :r ?y } .",
    ]);
    assert!(has(&c, "a", "p", iri("b")), "input fact carried; got {:?}", c);
    assert!(has(&c, "a", "q", iri("b")), "stratum-1 derivation carried; got {:?}", c);
    assert!(has(&c, "a", "r", iri("b")), "stratum-2 rule fired on carried fact; got {:?}", c);
    assert_eq!(sizes, vec![2, 3], "per-stratum term-level closure sizes");
}

#[test]
fn stratified_single_stratum_equals_reason_n3() {
    let src = "@prefix : <http://ex/> .\n:a :p :b .\n{ ?x :p ?y } => { ?x :q ?y } .";
    let (strat, sizes) = strat_closure(&[src]);
    assert_eq!(strat, closure(src));
    assert_eq!(sizes.len(), 1);
}

#[test]
fn stratified_negation_sees_earlier_strata_complete() {
    // The motivating soundness case (design doc §3.5): store-scoped NAF over a
    // DERIVED predicate. In a single document the negation can fire before the
    // membership rule derives, and the wrong conclusion persists; stratified,
    // stratum 2 sees the COMPLETE :Member extent.
    let s1 = "@prefix : <http://ex/> .\n\
        :a a :Candidate . :a :vouchedBy :b .\n\
        { ?x :vouchedBy ?y } => { ?x a :Member } .";
    let s2 = "@prefix : <http://ex/> .\n\
        @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
        { ?x a :Candidate . ?w log:notIncludes { ?x a :Member } . } => { ?x :rejected true } .";
    let (strat, _) = strat_closure(&[s1, s2]);
    assert!(
        !strat.iter().any(|(_, p, _)| p == &iri("rejected")),
        "stratified: :a IS a member, never rejected; got {:?}",
        strat
    );
    // The single document now agrees: the engine stratifies it automatically
    // (GH #6201), so the negation waits for the complete :Member extent. Before
    // that, the early firing persisted (derived facts are never retracted).
    let single = closure(&format!("{}\n{}", s1, s2));
    assert_eq!(single, strat, "auto-stratified single document = explicit strata");
}

#[test]
fn stratified_aggregation_over_a_derived_predicate() {
    // collectAllIn is sound over a predicate closed in an EARLIER stratum —
    // the ACP strata-B+C collapse shape.
    let s1 = "@prefix : <http://ex/> .\n\
        :g :member :m1 . :g :member :m2 .\n\
        { ?x :member ?m } => { ?x :grants ?m } .";
    let s2 = "@prefix : <http://ex/> .\n\
        @prefix math: <http://www.w3.org/2000/10/swap/math#> .\n\
        @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
        { ( ?m { :g :grants ?m } ?l ) log:collectAllIn ?w . ?l math:memberCount ?n . }\n\
          => { :g :grantCount ?n } .";
    let (c, _) = strat_closure(&[s1, s2]);
    assert!(has(&c, "g", "grantCount", int_lit(2)), "2 derived grants counted; got {:?}", c);
}

#[test]
fn stratified_blank_scope_is_per_stratum() {
    // A blank label reused across strata is a DISTINCT node per stratum,
    // exactly as re-parsing separate documents would behave.
    let (c, _) = strat_closure(&[
        "@prefix : <http://ex/> .\n_:b a :Thing .",
        "@prefix : <http://ex/> .\n_:b a :Other .",
    ]);
    let thing = c.iter().find(|(_, _, o)| o == &iri("Thing")).expect("Thing triple");
    let other = c.iter().find(|(_, _, o)| o == &iri("Other")).expect("Other triple");
    assert_ne!(thing.0, other.0, "carried _:b must stay distinct from stratum-2's _:b");
    assert_eq!(c.len(), 2, "exactly the two typing triples; got {:?}", c);
}

#[test]
fn stratified_carried_blank_survives_a_colliding_source_label() {
    // Regression: stratum 2's source explicitly uses `_:__st0_b` — the exact
    // label a FIXED `__st0_` carry rename would give stratum 1's `_:b`. The
    // carried node must be renamed into a namespace proven absent from the
    // stratum's own labels, keeping the two subjects distinct.
    let (c, _) = strat_closure(&[
        "@prefix : <http://ex/> .\n_:b a :Thing .",
        "@prefix : <http://ex/> .\n_:__st0_b a :Other .",
    ]);
    let thing = c.iter().find(|(_, _, o)| o == &iri("Thing")).expect("Thing triple");
    let other = c.iter().find(|(_, _, o)| o == &iri("Other")).expect("Other triple");
    assert_ne!(thing.0, other.0, "carried _:b captured by stratum-2's _:__st0_b; got {:?}", c);
    assert_eq!(c.len(), 2, "exactly the two typing triples; got {:?}", c);
}

#[test]
fn stratified_carried_existential_survives_a_colliding_source_label() {
    // The same collision, against a MINTED rule existential: stratum 1 derives
    // `:a :q _:e` with `_:e` skolemized to `__sk0_1_e`, and stratum 2's source
    // uses `_:__st0___sk0_1_e` — the label a fixed rename would have carried it
    // as. The carried existential must remain a distinct node.
    let (c, _) = strat_closure(&[
        "@prefix : <http://ex/> .\n:a :p :b .\n{ ?x :p ?y } => { ?x :q _:e } .",
        "@prefix : <http://ex/> .\n_:__st0___sk0_1_e a :Other .",
    ]);
    let minted =
        c.iter().find(|(s, p, _)| s == &iri("a") && p == &iri("q")).expect("minted :q triple");
    let other = c.iter().find(|(_, _, o)| o == &iri("Other")).expect("Other triple");
    assert_ne!(minted.2, other.0, "carried existential captured by stratum-2's label; got {:?}", c);
}
