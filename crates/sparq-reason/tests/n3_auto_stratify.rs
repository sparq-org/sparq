//! Automatic stratification of store-scoped negation-as-failure in ONE N3 document
//! (GH #6201, #5756).
//!
//! `log:notIncludes`, `log:collectAllIn` and `log:forAllIn` over the current store are
//! non-monotonic. Before stratification, a single fixpoint let a rule negate a predicate
//! that another rule of the same document had not derived yet, and the wrong derivation
//! persisted: an access decision failed OPEN. Each probe below derived the prohibited
//! fact on the pre-fix engine.

use sparq_core::dict::Dict;
use sparq_reason::n3::Term;
use sparq_reason::{
    reason_n3, reason_n3_query_terms, reason_n3_stratified, reason_n3_terms,
    reason_n3_terms_with_cycles, MaterializedN3Graph, N3Mode, NegationCycles,
};

const PRE: &str = "@prefix : <http://ex/> .\n\
    @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
    @prefix list: <http://www.w3.org/2000/10/swap/list#> .\n\
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n";

fn ex(l: &str) -> Term {
    Term::Iri(format!("http://ex/{l}"))
}

fn t(s: &str, p: &str, o: &str) -> [Term; 3] {
    [ex(s), ex(p), ex(o)]
}

fn run(body: &str) -> (Vec<[Term; 3]>, Vec<String>) {
    let c = reason_n3_terms(&format!("{PRE}{body}"), None).expect("reason_n3_terms");
    (c.facts, c.warnings)
}

fn run_with(body: &str, cycles: NegationCycles) -> (Vec<[Term; 3]>, Vec<String>) {
    let c = reason_n3_terms_with_cycles(&format!("{PRE}{body}"), None, None, cycles)
        .expect("reason_n3_terms_with_cycles");
    (c.facts, c.warnings)
}

fn rejected(body: &str) -> String {
    match reason_n3_terms(&format!("{PRE}{body}"), None) {
        Ok(c) => panic!("expected a stratification error; got {:?}", c.facts),
        Err(e) => e,
    }
}

/// A request :r against target :g, prohibited by :p; a second request :s on :h that no
/// prohibition covers.
const ACCESS_FACTS: &str = ":r :target :g . :p :prohibits :g . :s :target :h .\n\
    { ?r :target ?g . ?p :prohibits ?g } => { ?r :prohibitedIn ?g } .\n";

#[test]
fn collect_all_in_empty_guard_over_a_derived_predicate_does_not_fail_open() {
    // The lws-spec access-decision.n3 guard shape (#6201 / #5756).
    let (c, w) = run(&format!(
        "{ACCESS_FACTS}\
         {{ ?r :target ?g . ( true {{ ?r :prohibitedIn ?g }} ?LP ) log:collectAllIn _:sdp .\n\
            ?LP list:length 0 . }} => {{ ?r :permittedBy ?g }} ."
    ));
    assert!(
        c.contains(&t("r", "prohibitedIn", "g")),
        "prohibition derived; got {c:?}"
    );
    assert!(
        !c.contains(&t("r", "permittedBy", "g")),
        "prohibited request permitted: {c:?}"
    );
    assert!(
        c.contains(&t("s", "permittedBy", "h")),
        "unprohibited request permitted: {c:?}"
    );
    assert!(w.is_empty(), "stratifiable document must not warn: {w:?}");
}

#[test]
fn not_includes_guard_over_a_derived_predicate_does_not_fail_open() {
    let (c, w) = run(&format!(
        "{ACCESS_FACTS}\
         {{ ?r :target ?g . ?scope log:notIncludes {{ ?r :prohibitedIn ?g }} }}\n\
           => {{ ?r :allowedBy ?g }} ."
    ));
    assert!(c.contains(&t("r", "prohibitedIn", "g")));
    assert!(
        !c.contains(&t("r", "allowedBy", "g")),
        "prohibited request allowed: {c:?}"
    );
    assert!(
        c.contains(&t("s", "allowedBy", "h")),
        "unprohibited request allowed: {c:?}"
    );
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn for_all_in_over_a_derived_predicate_is_not_vacuously_true() {
    // Every flagged item must be approved; :x gets flagged by a rule and is never
    // approved, so the vacuous round-0 answer ("no flagged items yet") must not stick.
    let (c, _) = run(":x :risk :high .\n\
         { ?i :risk :high } => { ?i a :Flagged } .\n\
         { ( { ?i a :Flagged } { ?i a :Approved } ) log:forAllIn ?s } => { :batch :ok true } .");
    let rdf_type = Term::Iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type".into());
    assert!(c.contains(&[ex("x"), rdf_type, ex("Flagged")]), "{c:?}");
    assert!(
        !c.iter().any(|f| f[0] == ex("batch")),
        "unapproved flagged item passed the forAllIn gate: {c:?}"
    );
}

#[test]
fn multi_stratum_chain() {
    // Three strata: :flagged (positive) < :clean (negates :flagged) < :suspect (negates
    // :clean), plus a positive join in the top stratum over both lower strata.
    let (c, w) = run(":x :item true . :y :item true . :x :flag true .\n\
         { ?i :flag true } => { ?i :flagged true } .\n\
         { ?i :item true . ?s log:notIncludes { ?i :flagged true } } => { ?i :clean true } .\n\
         { ?i :item true . ?s log:notIncludes { ?i :clean true } } => { ?i :suspect true } .\n\
         { ?a :suspect true . ?b :clean true } => { ?a :contrast ?b } .");
    let yes = Term::Lit(
        "true".into(),
        "http://www.w3.org/2001/XMLSchema#boolean".into(),
        None,
    );
    let a = |s: &str, p: &str| [ex(s), ex(p), yes.clone()];
    assert!(c.contains(&a("x", "flagged")), "{c:?}");
    assert!(c.contains(&a("y", "clean")), "{c:?}");
    assert!(
        !c.contains(&a("x", "clean")),
        "x is flagged, never clean: {c:?}"
    );
    assert!(c.contains(&a("x", "suspect")), "{c:?}");
    assert!(
        !c.contains(&a("y", "suspect")),
        "y is clean, never suspect: {c:?}"
    );
    assert!(c.contains(&t("x", "contrast", "y")));
    assert_eq!(
        c.iter().filter(|f| f[1] == ex("contrast")).count(),
        1,
        "{c:?}"
    );
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn negation_over_one_class_of_rdf_type_is_stratified_by_class() {
    // `rdf:type` with a constant class is keyed by its class: `?i a :Clean` negating
    // `?i a :Flagged` is two strata, not a cycle.
    let (c, w) = run(":x a :Item . :x :flag true . :y a :Item .\n\
         { ?i :flag true } => { ?i a :Flagged } .\n\
         { ?i a :Item . ?s log:notIncludes { ?i a :Flagged } } => { ?i a :Clean } .");
    assert!(w.is_empty(), "{w:?}");
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    let typed = |s: &str, class: &str| [ex(s), Term::Iri(rdf_type.into()), ex(class)];
    assert!(c.contains(&typed("x", "Flagged")));
    assert!(
        !c.contains(&typed("x", "Clean")),
        "flagged item must not be clean"
    );
    assert!(c.contains(&typed("y", "Clean")));
    // The same rule as the second of two explicit strata (the documented workaround for
    // shapes the per-class analysis cannot separate) is accepted too.
    let s1 = format!(
        "{PRE}:x a :Item . :x :flag true . :y a :Item .\n\
         {{ ?i :flag true }} => {{ ?i a :Flagged }} ."
    );
    let s2 = format!(
        "{PRE}{{ ?i a :Item . ?s log:notIncludes {{ ?i a :Flagged }} }} => {{ ?i a :Clean }} ."
    );
    assert!(reason_n3_stratified(&mut Dict::new(), &[&s1, &s2]).is_ok());
}

#[test]
fn rdf_type_with_a_variable_class_still_conflicts_with_every_class() {
    // A conclusion `?i a ?c` may derive any class, including the negated one: a cycle.
    let e = rejected(
        ":x a :Item . :x :cls :Flagged .\n\
         { ?i a :Item . ?i :cls ?c . ?s log:notIncludes { ?i a :Flagged } } => { ?i a ?c } .",
    );
    assert!(e.contains("cycle"), "{e}");
    // A negation over `?i a ?c` reads every class, including one a rule derives: a cycle.
    let e = rejected(
        ":x a :Item .\n\
         { ?i a :Item . ?s log:notIncludes { ?i a ?c } } => { ?i a :Clean } .",
    );
    assert!(e.contains("cycle"), "{e}");
}

/// :r negates :q, and :q is derived from :r: a cycle through negation.
const CYCLE: &str = ":a :p :b .\n\
    { :a :p :b . ?s log:notIncludes { :a :q :b } } => { :a :r :b } .\n\
    { :a :r :b } => { :a :q :b } .";

#[test]
fn negation_cycle_is_rejected_by_default_everywhere() {
    let e = rejected(CYCLE);
    assert!(e.contains("<http://ex/q>") && e.contains("cycle"), "{e}");
    let src = format!("{PRE}{CYCLE}");
    assert!(reason_n3(&mut Dict::new(), &src).is_err());
    assert!(sparq_reason::reason_n3_proof(&mut Dict::new(), &src).is_err());
    assert!(sparq_reason::reason_n3_pass_all(&src, sparq_reason::RuleVars::N3).is_err());
    assert!(reason_n3_stratified(&mut Dict::new(), &[&src]).is_err());
    let q = format!("{PRE}{{ ?x :r ?y }} => {{ ?x :r ?y }} .");
    assert!(reason_n3_query_terms(&src, &q).is_err());
    let rules = format!(
        "{PRE}{{ :a :p :b . ?s log:notIncludes {{ :a :q :b }} }} => {{ :a :r :b }} .\n\
         {{ :a :r :b }} => {{ :a :q :b }} ."
    );
    assert!(MaterializedN3Graph::new(&rules, &[t("a", "p", "b")]).is_err());
}

#[test]
fn negation_cycle_opt_ins_report_the_cycle() {
    // Legacy single-pass keeps the pre-stratification result.
    let (c, w) = run_with(CYCLE, NegationCycles::SinglePass);
    assert!(
        c.contains(&t("a", "r", "b")) && c.contains(&t("a", "q", "b")),
        "{c:?}"
    );
    assert_eq!(w.len(), 1, "{w:?}");
    // Fail-closed derives nothing from the cyclic rules.
    let (c, w) = run_with(CYCLE, NegationCycles::FailClosed);
    assert!(
        !c.contains(&t("a", "r", "b")) && !c.contains(&t("a", "q", "b")),
        "{c:?}"
    );
    assert_eq!(w.len(), 1, "{w:?}");
}

#[test]
fn an_unrelated_cycle_does_not_unstratify_the_rest() {
    // A self-negating rule elsewhere in the document must not pull the access rules back
    // into one stratum.
    let body = format!(
        "{ACCESS_FACTS}\
         {{ ?r :target ?g . ?scope log:notIncludes {{ ?r :prohibitedIn ?g }} }}\n\
           => {{ ?r :allowedBy ?g }} .\n\
         {{ ?s log:notIncludes {{ :x :cycle true }} }} => {{ :x :cycle true }} ."
    );
    assert!(rejected(&body).contains("cycle"));
    let yes = Term::Lit(
        "true".into(),
        "http://www.w3.org/2001/XMLSchema#boolean".into(),
        None,
    );
    for mode in [NegationCycles::FailClosed, NegationCycles::SinglePass] {
        let (c, w) = run_with(&body, mode);
        assert!(
            !c.contains(&t("r", "allowedBy", "g")),
            "{mode:?}: prohibited request allowed"
        );
        assert!(c.contains(&t("s", "allowedBy", "h")), "{mode:?}: {c:?}");
        assert_eq!(w.len(), 1, "{w:?}");
        let cyc = c.contains(&[ex("x"), ex("cycle"), yes.clone()]);
        assert_eq!(cyc, mode == NegationCycles::SinglePass, "{mode:?}: {c:?}");
    }
}

#[test]
fn aggregation_clause_supplied_through_a_variable_fails_closed() {
    // The clause formula is a rule-produced value the analysis cannot pin to predicates:
    // it counts as negating every predicate, including the rule's own conclusion, so the
    // document is rejected rather than risk a premature permit.
    let body = ":r :target :g .\n\
         { :r :target :g } => { :policy :clause { :r :prohibitedIn :g } . :r :pending :g . } .\n\
         { ?r :pending ?g } => { ?r :prohibitedIn ?g } .\n\
         { :policy :clause ?C . ( true ?C ?L ) log:collectAllIn _:s . ?L list:length 0 . }\n\
           => { :r :permittedBy :g } .";
    assert!(rejected(body).contains("cycle"));
    let (c, w) = run_with(body, NegationCycles::FailClosed);
    assert!(
        !c.contains(&t("r", "permittedBy", "g")),
        "permit despite prohibition: {c:?}"
    );
    assert_eq!(w.len(), 1, "{w:?}");
}

#[test]
fn for_all_in_clause_supplied_through_a_variable_fails_closed() {
    let body = ":x :risk :high . :gate :when { ?i :flagged true } .\n\
         { ?i :risk :high } => { ?i :flagged true } .\n\
         { :gate :when ?A . ( ?A { ?i :approved true } ) log:forAllIn ?s } => { :batch :ok true } .";
    assert!(rejected(body).contains("cycle"));
    let (c, _) = run_with(body, NegationCycles::FailClosed);
    assert!(
        !c.iter().any(|f| f[0] == ex("batch")),
        "vacuous forAllIn: {c:?}"
    );
}

#[test]
fn list_builtin_over_a_derived_list_is_tracked() {
    let (c, w) = run(":seed :p :v .\n\
         { :seed :p :v } => { :h rdf:first :blocked . :h rdf:rest rdf:nil . } .\n\
         { :seed :p :v . ?scope log:notIncludes { :h list:member :blocked } . }\n\
           => { :r :permittedBy :g } .");
    assert!(
        !c.contains(&t("r", "permittedBy", "g")),
        "permit despite membership: {c:?}"
    );
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn variable_predicate_bound_through_a_virtual_list_is_unknown() {
    // ?p comes from first-class list access, not a stored triple: the conclusion's
    // predicate is unknown, so it may be :blocked and the negating rule waits for it.
    let (c, w) = run(":r :target :g .\n\
         { :r :target :g . (:blocked) rdf:first ?p } => { :r ?p :g } .\n\
         { :r :target :g . ?s log:notIncludes { :r :blocked :g } } => { :r :permittedBy :g } .");
    assert!(c.contains(&t("r", "blocked", "g")), "{c:?}");
    assert!(
        !c.contains(&t("r", "permittedBy", "g")),
        "permit despite :blocked: {c:?}"
    );
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn variable_predicate_conclusion_on_a_negation_cycle_is_rejected() {
    // A conclusion with a variable predicate may derive anything, including the predicate
    // its own premise negates.
    let body = ":Read :allowPred :read . :pol :allow :Read . :pol :appliesTo :doc .\n\
         { ?pol :allow ?m . ?m :allowPred ?pred . ?pol :appliesTo ?d .\n\
           ?s log:notIncludes { ?pol :noneOf ?x } } => { :alice ?pred ?d } .";
    let e = rejected(body);
    assert!(
        e.contains("cycle") && e.contains("variable predicate"),
        "{e}"
    );
}

#[test]
fn swap_namespace_predicate_that_is_not_a_builtin_is_ordinary() {
    // log:blocked is no builtin: it is a stored predicate like any other.
    let (c, w) = run(":r :target :g .\n\
         { :r :target :g } => { :r log:blocked :g } .\n\
         { :r :target :g . ?s log:notIncludes { :r log:blocked :g } } => { :r :permittedBy :g } .");
    assert!(
        !c.contains(&t("r", "permittedBy", "g")),
        "permit despite log:blocked: {c:?}"
    );
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn negation_cycle_in_a_nested_closure_fails_the_run() {
    // The nested document's rule negates its own conclusion. Suppressing it would leave
    // ?C without the prohibition and the outer negation would grant; the run must fail.
    let body = ":r :target :g .\n\
         { { { ?s log:notIncludes { :r :blocked :g } } => { :r :blocked :g } . }\n\
             log:conclusion ?C .\n\
           ?C log:notIncludes { :r :blocked :g } } => { :r :permittedBy :g } .";
    let e = rejected(body);
    assert!(e.contains("nested closure") && e.contains("cycle"), "{e}");
    // An explicit fail-closed run cannot fail closed through a nested closure: still an
    // error.
    let src = format!("{PRE}{body}");
    assert!(reason_n3_terms_with_cycles(&src, None, None, NegationCycles::FailClosed).is_err());
}

#[test]
fn negating_only_base_facts_is_one_stratum_and_unchanged() {
    let (c, w) = run(":a :p :b . :c :p :d . :c :blocked true .\n\
         { ?x :p ?y . ?s log:notIncludes { ?x :blocked true } } => { ?x :ok ?y } .");
    assert!(c.contains(&t("a", "ok", "b")));
    assert!(
        !c.iter().any(|f| f[0] == ex("c") && f[1] == ex("ok")),
        "{c:?}"
    );
    assert!(w.is_empty());
}

#[test]
fn formula_literal_scope_adds_no_dependency() {
    // A `{ … }` scope is local: negating :q inside it does not depend on the store, so
    // :q being derived elsewhere (even on a cycle) neither stratifies nor warns.
    let (c, w) = run(":a :p :b .\n\
         { :a :p :b . { :a :z :b } log:notIncludes { :a :q :b } } => { :a :r :b } .\n\
         { :a :r :b } => { :a :q :b } .");
    assert!(
        c.contains(&t("a", "r", "b")) && c.contains(&t("a", "q", "b")),
        "{c:?}"
    );
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn query_entry_point_sees_the_stratified_closure() {
    let data = format!(
        "{PRE}{ACCESS_FACTS}\
         {{ ?r :target ?g . ?scope log:notIncludes {{ ?r :prohibitedIn ?g }} }}\n\
           => {{ ?r :allowedBy ?g }} ."
    );
    let q = format!("{PRE}{{ ?r :allowedBy ?g }} => {{ ?r :allowedBy ?g }} .");
    let ans = reason_n3_query_terms(&data, &q).expect("query");
    assert_eq!(ans, vec![t("s", "allowedBy", "h")]);
}

#[test]
fn incremental_n3_falls_back_to_the_stratified_engine() {
    // The counting profile declines negation over a derived predicate and falls back to
    // the batch engine — which must now give the stratified answer.
    let rules = format!(
        "{PRE}{{ ?r :target ?g . ?p :prohibits ?g }} => {{ ?r :prohibitedIn ?g }} .\n\
         {{ ?r :target ?g . ?scope log:notIncludes {{ ?r :prohibitedIn ?g }} }}\n\
           => {{ ?r :allowedBy ?g }} ."
    );
    let base = [
        t("r", "target", "g"),
        t("p", "prohibits", "g"),
        t("s", "target", "h"),
    ];
    let mut g = MaterializedN3Graph::new(&rules, &base).expect("rules parse");
    assert_eq!(g.mode(), N3Mode::Fallback);
    assert!(!g.contains(&t("r", "allowedBy", "g")));
    assert!(g.contains(&t("s", "allowedBy", "h")));
    // A mutation re-materializes through the same engine.
    g.insert(&[t("p", "prohibits", "h")]);
    assert!(!g.contains(&t("s", "allowedBy", "h")));
    assert!(g.contains(&t("s", "prohibitedIn", "h")));
}

/// A self-negating backward rule supplied by the QUERY document (not the data document)
/// must be refused like one in the data: the evaluated rule set is data plus query.
#[test]
fn query_supplied_backward_negation_cycle_is_rejected_at_both_query_entry_points() {
    let query = format!(
        "{PRE}{{ :r :blocked :g }} <= {{ ?s log:notIncludes {{ :r :blocked :g }} }} .\n\
         {{ ?s log:notIncludes {{ :r :blocked :g }} }} => {{ :r :permittedBy :g }} ."
    );
    let e = reason_n3_query_terms(PRE, &query).expect_err("query-supplied cycle refused");
    assert!(
        e.contains("cycle") && e.contains("<http://ex/blocked>"),
        "{e}"
    );
    assert!(sparq_reason::reason_n3_query(&mut Dict::new(), PRE, &query).is_err());
    // The same backward rule split across the two documents is refused too.
    let data =
        format!("{PRE}{{ :r :blocked :g }} <= {{ ?s log:notIncludes {{ :r :blocked :g }} }} .");
    let q = format!("{PRE}{{ :r :blocked :g }} => {{ :r :seen :g }} .");
    assert!(reason_n3_query_terms(&data, &q).is_err());
}

/// A query rule's conclusions are projected, never added to the store, so a query that
/// concludes a predicate the data negates is no cycle.
#[test]
fn query_conclusions_do_not_feed_back_into_the_analysis() {
    let data = format!(
        "{PRE}:a :p :b .\n{{ :a :p :b . ?s log:notIncludes {{ :a :q :b }} }} => {{ :a :r :b }} ."
    );
    let q = format!("{PRE}{{ :a :r :b }} => {{ :a :q :b }} .");
    let out = reason_n3_query_terms(&data, &q).expect("no cycle through a projection");
    assert_eq!(out, vec![t("a", "q", "b")]);
}

/// A chain of 65 backward rules `{ :a :pN :b } <= { :a :p(N+1) :b }` over the fact
/// `:a :p65 :b`: proving `:a :p0 :b` needs one more rule application than the backward
/// search depth allows.
fn deep_chain() -> String {
    let mut s = String::from(":a :p65 :b .\n");
    for n in 0..65 {
        s.push_str(&format!("{{ :a :p{n} :b }} <= {{ :a :p{} :b }} .\n", n + 1));
    }
    s
}

/// A negation over the truncated proof must not read the cut as absence.
#[test]
fn truncated_backward_proof_cannot_authorize_through_negation() {
    let body = format!(
        "{}{{ ?s log:notIncludes {{ :a :p0 :b }} }} => {{ :r :permittedBy :g }} .",
        deep_chain()
    );
    let e = rejected(&body);
    assert!(e.contains("depth limit"), "{e}");
    // The query entry points, with the chain in the data and the negation in the query.
    let data = format!("{PRE}{}", deep_chain());
    let q = format!("{PRE}{{ ?s log:notIncludes {{ :a :p0 :b }} }} => {{ :r :permittedBy :g }} .");
    let e = reason_n3_query_terms(&data, &q).expect_err("query refuses");
    assert!(e.contains("depth limit"), "{e}");
    assert!(sparq_reason::reason_n3_query(&mut Dict::new(), &data, &q).is_err());
    // The explicit opt-in keeps the legacy (incomplete) answer.
    let (c, _) = run_with(&body, NegationCycles::SinglePass);
    assert!(c.contains(&t("r", "permittedBy", "g")));
}

/// Every negation and aggregation builtin, through every entry point, over an input
/// whose backward proof search is cut short: each is refused (an error, never an answer
/// that read the cut as absence).
#[test]
fn every_negation_and_aggregation_refuses_a_truncated_input_at_every_entry_point() {
    // Rule 0 runs first and cuts its search; the probe rule then negates or aggregates.
    let trigger = "{ :a :p0 :b } => { :a :proved :b } .\n";
    let probes: [(&str, &str); 6] = [
        (
            "notIncludes, store scope",
            "?s log:notIncludes { :a :p0 :b }",
        ),
        (
            "notIncludes, formula scope",
            "{ :c :d :e } log:notIncludes { :a :p0 :b }",
        ),
        (
            "notIncludes, empty formula scope",
            "{} log:notIncludes { :a :p0 :b }",
        ),
        (
            "collectAllIn, store scope",
            "( ?x { :a :p0 ?x } ?l ) log:collectAllIn ?s",
        ),
        (
            "forAllIn, store scope",
            "( { :a :p0 ?x } { :a :never ?x } ) log:forAllIn ?s",
        ),
        (
            "collectAllIn, formula scope",
            "( ?x { :c :d ?x } ?l ) log:collectAllIn { :c :d :e }",
        ),
    ];
    for (what, probe) in probes {
        let rules = format!("{trigger}{{ {probe} }} => {{ :r :permittedBy :g }} .");
        let body = format!("{}{rules}", deep_chain());
        let src = format!("{PRE}{body}");
        let refused = |r: Result<(), String>, entry: &str| match r {
            Err(e) => assert!(e.contains("incomplete"), "{what} / {entry}: {e}"),
            Ok(()) => panic!("{what} / {entry}: accepted a truncated input"),
        };
        refused(reason_n3(&mut Dict::new(), &src).map(drop), "reason_n3");
        refused(reason_n3_terms(&src, None).map(drop), "reason_n3_terms");
        for cycles in [NegationCycles::Reject, NegationCycles::FailClosed] {
            refused(
                reason_n3_terms_with_cycles(&src, None, None, cycles).map(drop),
                "reason_n3_terms_with_cycles",
            );
        }
        refused(
            sparq_reason::reason_n3_proof(&mut Dict::new(), &src).map(drop),
            "proof",
        );
        refused(
            sparq_reason::reason_n3_pass_all(&src, sparq_reason::RuleVars::N3).map(drop),
            "pass_all",
        );
        refused(
            reason_n3_stratified(&mut Dict::new(), &[&src]).map(drop),
            "stratified",
        );
        // Query: the chain is data, the trigger and the probe are the query's premise.
        let data = format!("{PRE}{}", deep_chain());
        let q = format!("{PRE}{{ :a :p0 :b }} => {{ :a :proved :b }} .\n{{ {probe} }} => {{ :r :permittedBy :g }} .");
        refused(reason_n3_query_terms(&data, &q).map(drop), "query_terms");
        refused(
            sparq_reason::reason_n3_query(&mut Dict::new(), &data, &q).map(drop),
            "query",
        );
        // Nested closure: the whole document inside log:conclusion, negation inside.
        let nested = format!(
            "{PRE}{{ {{ {body} }} log:conclusion ?c . ?c log:notIncludes {{ :r :x :y }} }} => {{ :r :ok :g }} ."
        );
        refused(
            reason_n3_terms(&nested, None).map(drop),
            "nested log:conclusion",
        );
        // Incremental: the fallback runs the checked engine; construction refuses.
        refused(
            MaterializedN3Graph::new(&src, &[]).map(drop),
            "MaterializedN3Graph::new",
        );
        // Compiled: backward rules (the truncation source) are outside the subset.
        #[cfg(feature = "compiled-rules")]
        assert!(
            sparq_reason::n3::compiled::compile(&src).is_err(),
            "{what} / compiled"
        );
    }
}
