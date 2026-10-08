//! Automatic stratification of store-scoped negation-as-failure in ONE N3 document
//! (GH #6201, #5756).
//!
//! `log:notIncludes`, `log:collectAllIn` and `log:forAllIn` over the current store are
//! non-monotonic. Before stratification, a single fixpoint let a rule negate a predicate
//! that another rule of the same document had not derived yet, and the wrong derivation
//! persisted: an access decision failed OPEN. Each probe below derived the prohibited
//! fact on the pre-fix engine.

use sparq_reason::n3::Term;
use sparq_reason::{reason_n3_query_terms, reason_n3_terms, MaterializedN3Graph, N3Mode};

const PRE: &str = "@prefix : <http://ex/> .\n\
    @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
    @prefix list: <http://www.w3.org/2000/10/swap/list#> .\n";

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
fn negation_over_one_class_of_a_shared_predicate_is_a_cycle() {
    // Dependencies are tracked per PREDICATE: `?i a :Clean` negating `?i a :Flagged`
    // negates rdf:type from a rule that concludes rdf:type, which is a cycle through
    // negation. The document stays single-pass and says so.
    let (_, w) = run(":x a :Item . :x :flag true .\n\
         { ?i :flag true } => { ?i a :Flagged } .\n\
         { ?i a :Item . ?s log:notIncludes { ?i a :Flagged } } => { ?i a :Clean } .");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("22-rdf-syntax-ns#type"), "{w:?}");
}

#[test]
fn negation_cycle_keeps_single_pass_result_and_warns() {
    // :r negates :q, and :q is derived from :r — a cycle through negation, not
    // stratifiable. The single-pass result is kept (round 0 fires the negation, round 1
    // derives :q; nothing is retracted) and a diagnostic names the predicate.
    let (c, w) = run(":a :p :b .\n\
         { :a :p :b . ?s log:notIncludes { :a :q :b } } => { :a :r :b } .\n\
         { :a :r :b } => { :a :q :b } .");
    assert!(c.contains(&t("a", "r", "b")), "{c:?}");
    assert!(c.contains(&t("a", "q", "b")), "{c:?}");
    assert_eq!(w.len(), 1, "one stratification diagnostic: {w:?}");
    assert!(
        w[0].contains("<http://ex/q>") && w[0].contains("cycle"),
        "{w:?}"
    );
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
