//! GH #6754: a formula-level `@forAll` binds only in the formula that declares it.
//!
//! The parser names such a universal by its declared IRI plus its de Bruijn distance to
//! the declaring formula (`?__uf.<n>.<iri>`), so the name is unforgeable (a `.` cannot
//! occur in a source quickvar) and canonical (alpha-equivalent copies of a formula are the
//! same term). Rules with premise-local universals are rejected; every other formula-level
//! universal belongs to its quoted formula, matches only itself and is never substituted. Document-level `@forAll` is unchanged.

use sparq_reason::n3::{parser, Term};
use sparq_reason::{reason_n3_pass_all, reason_n3_terms, reason_n3_terms_with_cycles, NegationCycles, RuleVars};

const EX: &str = "http://ex/";

fn iri(local: &str) -> Term {
    Term::Iri(format!("{EX}{local}"))
}

fn closure(src: &str) -> Vec<[Term; 3]> {
    reason_n3_terms(&format!("@prefix : <{EX}>.\n{src}"), None).expect("reasons").facts
}

fn has(facts: &[[Term; 3]], s: &str, p: &str, o: &str) -> bool {
    facts.contains(&[iri(s), iri(p), iri(o)])
}

/// Every variable name in `t`, at any depth.
fn vars(t: &Term, out: &mut Vec<String>) {
    match t {
        Term::Var(v) => out.push(v.clone()),
        Term::List(ms) => ms.iter().for_each(|m| vars(m, out)),
        Term::Triple(tr) => tr.iter().for_each(|m| vars(m, out)),
        Term::Formula(ts) => ts.iter().flatten().for_each(|m| vars(m, out)),
        _ => {}
    }
}

// These entry-point tests exercise validate_rule_quantifiers in every engine.
const QUANTIFIER_ERROR: &str = "N3: universally quantified antecedents are unsupported (@forAll in a rule premise or scoped body pattern)";

fn premise_cases() -> Vec<String> {
    [
        "{ @forAll :x. :x :p :b } => { @forAll :x. :x :q :b }. :c :p :b.",
        "@forAll :y. :y :p :b. { @forAll :x. :x :p :b } => { :hit :by :rule }.",
        "@forAll :z. :z :p :z. { @forAll :x, :y. :x :p :y } => { :hit :by :rule }.",
        // An ordinary variable must not alias a premise-local universal.
        "@forAll :z. :z :p :z. { @forAll :x. :x :p ?y } => { :hit :by :rule }.",
        "{ @forAll :x. ( 1 2 ) math:sum :x } => { :hit :by :rule }.",
        "{ @forAll :x. :x :p :b } => { ?__uf0_x :q :b }.",
        "{ @forAll :x. :x :p :b } => { ?__up_x :q :b }.",
        // Backward rules have the same unsupported antecedent.
        "{ :hit :by :rule } <= { @forAll :x. :x :p :b }.",
    ].iter().map(|src| format!("@prefix : <{EX}>. {src}")).collect()
}

fn scope_cases() -> Vec<String> {
    ["notIncludes", "includes", "supports"].iter().flat_map(|op| {
        [
            format!("{{ ?scope log:{op} {{ @forAll :x. :x :p :b }} }} => {{ :hit :by :rule }}."),
            // A pattern's own universal mentioned inside a nested formula of the pattern.
            format!("{{ ?scope log:{op} {{ @forAll :x. :a :says {{ :x :p :b }} }} }} => {{ :hit :by :rule }}."),
            // A quantified pattern nested in an aggregation clause.
            format!("{{ ( :t {{ ?s log:{op} {{ @forAll :x. :x :p :b }} }} ?l ) log:collectAllIn ?scope }} => {{ :hit :by :rule }}."),
        ]
    }).chain([
        "{ ( ?x { @forAll :x. :x :p :b } ?list ) log:collectAllIn ?scope } => { :hit :by :rule }.".into(),
        "{ ( { @forAll :x. :x :p :b } { :a :p :b } ) log:forAllIn ?scope } => { :hit :by :rule }.".into(),
        "{ ( { :a :p :b } { @forAll :x. :x :p :b } ) log:forAllIn ?scope } => { :hit :by :rule }.".into(),
        // A quantified CLAUSE of an aggregation nested in another aggregation's clause.
        "{ ( ?inner { ( :t { @forAll :x. :x :p :b } ?inner ) log:collectAllIn ?scope } ?outer ) \
           log:collectAllIn ?scope } => { :hit :by :rule }.".into(),
        "{ ( { ( { @forAll :x. :x :p :b } { :a :p :b } ) log:forAllIn ?scope } { :a :p :b } ) \
           log:forAllIn ?scope } => { :hit :by :rule }.".into(),
        // A clause's own universal mentioned in a template nested in it: the clause binds it.
        "{ ( ?inner { @forAll :x. :x :p :b. ( { :x :q :b } { :a :p :b } ?inner ) log:collectAllIn ?scope } ?outer ) \
           log:collectAllIn ?scope } => { :hit :by :rule }.".into(),
    ]).map(|src| format!("@prefix : <{EX}>. {src}")).collect()
}

#[test]
fn naive_rejects_premise_own_forall() {
    for src in premise_cases() {
        assert_eq!(reason_n3_terms(&src, None).err().expect("rejected"), QUANTIFIER_ERROR, "{src}");
    }
}

#[test]
fn naive_rejects_forall_in_scope_patterns() {
    for src in scope_cases() {
        assert_eq!(reason_n3_terms(&src, None).err().expect("rejected"), QUANTIFIER_ERROR, "{src}");
    }
}

#[cfg(feature = "compiled-rules")]
#[test]
fn compiled_rejects_premise_own_forall() {
    for src in premise_cases() {
        assert_eq!(sparq_reason::n3::compiled::compile(&src).unwrap_err(), QUANTIFIER_ERROR, "{src}");
    }
}

#[cfg(feature = "compiled-rules")]
#[test]
fn compiled_rejects_forall_in_scope_patterns() {
    for src in scope_cases() {
        assert_eq!(sparq_reason::n3::compiled::compile(&src).unwrap_err(), QUANTIFIER_ERROR, "{src}");
    }
}

#[test]
fn incremental_rejects_premise_own_forall() {
    for src in premise_cases() {
        let err = sparq_reason::MaterializedN3Graph::new(&src, &[]).err().expect("rejected");
        assert_eq!(err, QUANTIFIER_ERROR, "{src}");
    }
}

#[test]
fn incremental_rejects_forall_in_scope_patterns() {
    for src in scope_cases() {
        let err = sparq_reason::MaterializedN3Graph::new(&src, &[]).err().expect("rejected");
        assert_eq!(err, QUANTIFIER_ERROR, "{src}");
    }
}

#[test]
fn a_query_rejects_premise_own_forall() {
    let src = format!("@prefix : <{EX}>. {{ @forAll :x. :x :p :b }} => {{ :hit :by :rule }}.");
    assert_eq!(sparq_reason::reason_n3_query_terms("", &src).unwrap_err(), QUANTIFIER_ERROR);
}

#[test]
fn a_nested_closure_rejects_premise_own_forall() {
    // An inner premise cannot use the enclosing formula's lifted universal as its own.
    let src = format!("@prefix : <{EX}>.
        :k :is {{ @forAll :x. :x :p :b. {{ @forAll :y. :y :p :b }} => {{ :hit :by :rule }} }}.
        {{ :k :is ?f. ?f log:conclusion ?g }} => {{ :result :is ?g }}.");
    assert_eq!(reason_n3_terms(&src, None).err().expect("rejected"), format!("in a nested closure: {QUANTIFIER_ERROR}"));
}

#[test]
fn a_quoted_conclusion_keeps_its_own_binder() {
    // The quoted conclusion's binder is independent of the ordinary premise variable.
    let src = "{ ?x :p :b } => { :log :says { @forAll :x. :x :p :b } }.\n:c :p :b.\n\
               :want :is { @forAll :x. :x :p :b }.\n";
    let facts = closure(src);
    let want = facts.iter().find(|f| f[0] == iri("want")).expect("source fact")[2].clone();
    let said: Vec<&Term> = facts.iter().filter(|f| f[0] == iri("log")).map(|f| &f[2]).collect();
    assert_eq!(said, vec![&want], "one firing, quoting the formula unchanged: {facts:?}");
}

#[test]
fn copies_of_one_closed_formula_are_equal() {
    let src = ":a :p { @forAll :x. :x :q :z }.\n:b :p { @forAll :x. :x :q :z }.\n\
               { :a :p ?f. :b :p ?g. ?f log:equalTo ?g } => { :same :is :yes }.\n\
               { :a :p ?f. :b :p ?g. ?f log:notEqualTo ?g } => { :differ :is :yes }.\n";
    let facts = closure(src);
    assert!(has(&facts, "same", "is", "yes"), "{facts:?}");
    assert!(!has(&facts, "differ", "is", "yes"), "{facts:?}");
    // Nested: an inner mention of the outer binder is the same term in both copies.
    let src = ":a :p { @forAll :x. :x :q { :x :r :z } }.\n:b :p { @forAll :x. :x :q { :x :r :z } }.\n\
               { :a :p ?f. :b :p ?g. ?f log:equalTo ?g } => { :same :is :yes }.\n";
    assert!(has(&closure(src), "same", "is", "yes"));
    // And a quoted copy written as a rule pattern matches the fact.
    let src = ":a :p { @forAll :x. :x :q :z }.\n{ :a :p { @forAll :x. :x :q :z } } => { :quoted :match :yes }.\n";
    assert!(has(&closure(src), "quoted", "match", "yes"));
}

#[test]
fn a_nested_declaration_shadows_the_outer_one() {
    // `{ :x :r :z }` mentions the OUTER `:x` in the first formula and its OWN `:x` in the
    // second, so the two formulae differ (main read both as one variable: equal).
    let src = ":a :p { @forAll :x. :x :q { :x :r :z } }.\n\
               :b :p { @forAll :x. :x :q { @forAll :x. :x :r :z } }.\n\
               { :a :p ?f. :b :p ?g. ?f log:notEqualTo ?g } => { :shadow :distinct :yes }.\n";
    assert!(has(&closure(src), "shadow", "distinct", "yes"));
    let p = parser::parse(&format!("@prefix : <{EX}>.\n{}", src.lines().take(2).collect::<String>())).unwrap();
    let mut a = Vec::new();
    vars(&p.facts[0][2], &mut a);
    let mut b = Vec::new();
    vars(&p.facts[1][2], &mut b);
    assert_eq!(a[0], b[0], "the outer binder at the outer formula's top level");
    assert_ne!(a[1], b[1], "outer binder vs the inner formula's own binder: {a:?} {b:?}");
}

#[test]
fn a_quoted_universal_in_a_premise_matches_only_itself() {
    // `{ @forAll :x. :x :q :z }` in a premise is a closed quoted formula: it is not the
    // ground `{ :k :q :z }`, and a rule variable cannot bind to its bound `:x`.
    let src = ":a :says { :k :q :z }.\n{ :a :says { @forAll :x. :x :q :z } } => { :matched :ground :yes }.\n";
    assert!(!has(&closure(src), "matched", "ground", "yes"));
    let src = ":a :says { @forAll :x. :x :q :z }.\n{ :a :says { ?s :q :z } } => { :w :saw { ?s :r :z } }.\n";
    let facts = closure(src);
    assert!(!facts.iter().any(|f| f[0] == iri("w")), "a bound universal does not escape: {facts:?}");
}

#[test]
fn a_formula_s_own_universals_quantify_its_rules_under_log_conclusion() {
    let src = ":k :is { @forAll :x. :c :p :b. { :x :p :b } => { :x :q :b } }.\n\
               { :k :is ?f. ?f log:conclusion ?g. ?g log:includes { :c :q :b } } => { :closure :fires :yes }.\n";
    assert!(has(&closure(src), "closure", "fires", "yes"));
}

#[test]
fn a_nested_closure_keeps_the_enclosing_formula_s_universals() {
    // The inner closure mentions the OUTER formula's :x; it must not claim that binder.
    let src = ":k :is { @forAll :x. :a :says { :x :q :z }. { :a :says ?g. ?g log:conclusion ?h } => { :a :closed ?h } }.\n\
               { :k :is ?f. ?f log:conclusion ?c } => { :result :is ?c }.\n";
    let facts = closure(src);
    let result = facts.iter().find(|f| f[0] == iri("result")).expect("closed result");
    let Term::Formula(rows) = &result[2] else { panic!("formula result: {result:?}") };
    assert!(rows.contains(&[iri("a"), iri("closed"), Term::Formula(vec![
        [Term::Var(format!("__uf.1.{EX}x")), iri("q"), iri("z")]
    ])]), "{rows:?}");
}

#[test]
fn a_closure_keeps_universals_its_rules_assert_in_the_result_formula() {
    let src = ":k :is { :c :p :b. { :c :p :b } => { @forAll :x. :a :says { :x :q :z } } }.\n\
               { :k :is ?f. ?f log:conclusion ?c } => { :result :is ?c }.\n\
               { :k :is ?f. ?f log:supports { :a :says _:formula } } => { :supports :scoped :ok }.\n";
    let facts = closure(src);
    let result = facts.iter().find(|f| f[0] == iri("result")).expect("closed result");
    let Term::Formula(rows) = &result[2] else { panic!("formula result: {result:?}") };
    assert!(rows.contains(&[iri("a"), iri("says"), Term::Formula(vec![
        [Term::Var(format!("__uf.1.{EX}x")), iri("q"), iri("z")]
    ])]), "{rows:?}");
    assert!(has(&facts, "supports", "scoped", "ok"), "{facts:?}");
}

#[test]
fn document_level_for_all_is_unchanged() {
    let facts = closure("@forAll :x. { :x :p :b } => { :x :q :b }.\n:c :p :b.\n");
    assert!(has(&facts, "c", "q", "b"), "{facts:?}");
    let p = parser::parse(&format!("@prefix : <{EX}>. @forAll :x. :a :p {{ :x :q :z }}. {{ :x :p :b }} => {{ :x :q :b }}.")).unwrap();
    let want = Term::Var(format!("__ua.{EX}x"));
    let mut v = Vec::new();
    vars(&p.facts[0][2], &mut v);
    assert_eq!(v, vec![format!("__ua.{EX}x")]);
    assert_eq!(p.rules[0].premise[0][0], want);
    assert_eq!(p.rules[0].conclusion[0][0], want);
}

#[test]
fn no_generated_binder_reaches_a_derived_fact() {
    let src = "{ ?x :p :b } => { @forAll :z. :z :q :b. :r :s { @forAll :y. :y :t ?x } }.\n:c :p :b.\n";
    let facts = closure(src);
    assert!(facts.iter().any(|f| f[0] == iri("r")), "the rule fired: {facts:?}");
    for f in &facts {
        let mut v = Vec::new();
        f.iter().for_each(|t| vars(t, &mut v));
        assert!(v.iter().all(|n| !n.starts_with("__up.") && !n.starts_with("__uc.")), "{f:?}");
    }
}

#[test]
fn formula_level_universals_round_trip_through_pass_all() {
    let src = format!(
        "@prefix : <{EX}>.\n:a :p {{ @forAll :x. :x :q {{ @forAll :x. :x :r :z }} }}.\n\
         {{ ?x :p :b }} => {{ @forAll :x. :x :q :b. :log :says {{ @forAll :x. :x :p :b }} }}.\n:c :p :b.\n"
    );
    let doc = reason_n3_pass_all(&src, RuleVars::N3).expect("pass-all writes formula-level universals");
    assert!(!doc.contains("__u"), "no engine-internal name: {doc}");
    assert!(doc.contains("{ @forAll <http://ex/x> ."), "declared inside its formula: {doc}");
    let (a, b) = (parser::parse(&src).unwrap(), parser::parse(&doc).unwrap());
    assert_eq!(a.rules.len(), b.rules.len());
    assert_eq!(a.rules[0].premise, b.rules[0].premise, "{doc}");
    assert_eq!(a.rules[0].conclusion, b.rules[0].conclusion, "{doc}");
    assert_eq!(doc, reason_n3_pass_all(&doc, RuleVars::N3).expect("round two"));
}

/// Aggregation clauses reached through a variable: [`validate_rule_quantifiers`] cannot see
/// them in the rule text, so `log:forAllIn` / `log:collectAllIn` check the bound clause
/// before evaluating it and fail the run with the same error (a quantified clause would
/// otherwise have no solutions and `log:forAllIn` would hold vacuously).
fn bound_clause_cases() -> Vec<String> {
    [
        // A formula scope: a store scope over a variable clause is already refused by
        // stratification (it cannot pin the clause's predicates).
        // The whole clause list is a variable's value.
        ":cfg :clauses ( { @forAll :x. :x :p :b } { :a :p :b } ). \
         { :cfg :clauses ?cs. ?cs log:forAllIn { :a :p :b } } => { :hit :by :rule }.",
        ":cfg :clauses ( { :a :p :b } { @forAll :x. :x :p :b } ). \
         { :cfg :clauses ?cs. ?cs log:forAllIn { :a :p :b } } => { :hit :by :rule }.",
        // A variable clause inside a written list.
        ":cfg :clause { @forAll :x. :x :p :b }. \
         { :cfg :clause ?c. ( ?c { :a :p :b } ) log:forAllIn { :a :p :b } } => { :hit :by :rule }.",
        ":cfg :clause { @forAll :x. :x :p :b }. \
         { :cfg :clause ?c. ( :t ?c ?l ) log:collectAllIn { :a :p :b } } => { :got :list ?l }.",
        // A variable subject.
        ":cfg :query ( :t { @forAll :x. :x :p :b } :out ). \
         { :cfg :query ?q. ?q log:collectAllIn { :a :p :b } } => { :hit :by :rule }.",
    ]
    .iter()
    .map(|src| format!("@prefix : <{EX}>. :a :p :b. {src}"))
    .collect()
}

#[test]
fn a_bound_quantified_aggregation_clause_is_rejected() {
    for src in bound_clause_cases() {
        assert_eq!(reason_n3_terms(&src, None).err().expect("rejected"), QUANTIFIER_ERROR, "{src}");
    }
}

#[test]
fn a_bound_quantified_aggregation_clause_fails_the_incremental_graph() {
    for src in bound_clause_cases() {
        // The graph reasons over base facts given as terms; the document's facts go there.
        let base = parser::parse(&src).expect("parses").facts;
        // Its first materialization reaches the clause and refuses the document.
        let err = sparq_reason::MaterializedN3Graph::new(&src, &base).err().expect("rejected");
        assert!(err.starts_with(QUANTIFIER_ERROR), "{src}: {err}");
    }
}

/// The `log:collectAllIn` template and a value bound into a written clause are values, not
/// matching clauses: a closed quantified formula there keeps identity matching.
#[test]
fn a_quantified_template_or_bound_value_is_collected() {
    let quoted = |facts: &[[Term; 3]], s: &str, p: &str| {
        facts.iter().any(|f| {
            f[0] == iri(s)
                && f[1] == iri(p)
                && matches!(&f[2], Term::List(ms) if matches!(&ms[..], [Term::Formula(ts)] if ts.len() == 1))
        })
    };
    let facts = closure(
        ":a :p :b. { ( { @forAll :x. :x :q :b } { :a :p :b } ?l ) log:collectAllIn ?scope } => { :got :list ?l }.",
    );
    assert!(quoted(&facts, "got", "list"), "{facts:?}");
    let facts = closure(
        ":a :says { @forAll :x. :x :q :z }. { ( ?f { :a :says ?f } ?l ) log:collectAllIn ?scope } => { :got :saw ?l }.",
    );
    assert!(quoted(&facts, "got", "saw"), "{facts:?}");
}

/// A `log:includes` / `log:notIncludes` pattern reached through a variable: a quantified
/// formula bound to it is refused when the builtin is evaluated, as a written one is.
fn bound_pattern_cases(pattern: &str) -> Vec<String> {
    ["includes", "notIncludes"]
        .iter()
        .flat_map(|op| {
            [
                format!("{{ :cfg :pat ?f. {{ :a :p :b }} log:{op} ?f }} => {{ :hit :by :rule }}."),
                // The store as the scope.
                format!("{{ :cfg :pat ?f. ?scope log:{op} ?f }} => {{ :hit :by :rule }}."),
            ]
        })
        .map(|rule| format!("@prefix : <{EX}>. :a :p :b. :cfg :pat {pattern}. {rule}"))
        .collect()
}

/// A store-scoped `log:notIncludes` of a bound pattern is a negation cycle with its own
/// conclusion (GH #6936): the default refuses it before it runs, so these cases opt in to
/// `SinglePass` to reach the builtin.
fn store_negation(src: &str) -> bool {
    src.contains("?scope log:notIncludes")
}

#[test]
fn a_bound_quantified_containment_pattern_is_rejected() {
    for src in bound_pattern_cases("{ @forAll :x. :x :p :b }") {
        let base = parser::parse(&src).expect("parses").facts;
        let graph = sparq_reason::MaterializedN3Graph::new(&src, &base).err().expect("rejected");
        if store_negation(&src) {
            let err = reason_n3_terms(&src, None).err().expect("rejected");
            assert!(err.contains("stratification"), "{src}: {err}");
            let err = reason_n3_terms_with_cycles(&src, None, None, NegationCycles::SinglePass).err().expect("rejected");
            assert_eq!(err, QUANTIFIER_ERROR, "{src}");
            assert!(graph.contains("stratification"), "{src}: {graph}");
            continue;
        }
        assert_eq!(reason_n3_terms(&src, None).err().expect("rejected"), QUANTIFIER_ERROR, "{src}");
        assert!(graph.starts_with(QUANTIFIER_ERROR), "{src}: {graph}");
    }
}

/// A bound pattern without a formula-level universal is not refused.
#[test]
fn a_bound_unquantified_containment_pattern_is_not_refused() {
    for src in bound_pattern_cases("{ :c :p :b }") {
        let base = parser::parse(&src).expect("parses").facts;
        if store_negation(&src) {
            assert!(reason_n3_terms_with_cycles(&src, None, None, NegationCycles::SinglePass).is_ok(), "{src}");
            continue;
        }
        assert!(reason_n3_terms(&src, None).is_ok(), "{src}");
        let g = sparq_reason::MaterializedN3Graph::new(&src, &base).expect("accepted");
        assert_eq!(g.stratification_warning(), None, "{src}");
    }
}

/// GH #6851 review: a template stays a value however deep its aggregation sits — here
/// inside another aggregation's clause — so its own universal is allowed there too.
#[test]
fn a_quantified_template_nested_in_a_clause_is_collected() {
    let src = ":a :p :b.
        {
          ( ?inner {
              ( { @forAll :x. :x :q :b } { :a :p :b } ?inner ) log:collectAllIn ?scope
            } ?outer ) log:collectAllIn ?scope
        } => { :got :list ?outer }.";
    let facts = closure(src);
    let template = parser::parse(&format!("@prefix : <{EX}>. :t :is {{ @forAll :x. :x :q :b }}."))
        .expect("parses")
        .facts[0][2]
        .clone();
    let want = [iri("got"), iri("list"), Term::List(vec![Term::List(vec![template])])];
    assert!(facts.contains(&want), "{facts:?}");
    let base = parser::parse(&format!("@prefix : <{EX}>. {src}")).expect("parses").facts;
    assert!(sparq_reason::MaterializedN3Graph::new(&format!("@prefix : <{EX}>. {src}"), &base).is_ok());
}

/// A containment pattern is a clause, but a formula quoted INSIDE it is a value, matched
/// by identity like a quoted formula anywhere in a premise: its own universal is allowed.
#[test]
fn a_quoted_formula_inside_a_containment_pattern_matches_by_identity() {
    let facts = closure(
        ":k :is { :a :says { @forAll :x. :x :p :b } }.
         { :k :is ?f. ?f log:includes { :a :says { @forAll :x. :x :p :b } } } => { :hit :by :identity }.
         { :k :is ?f. ?f log:includes { :a :says { :c :p :b } } } => { :hit :by :instance }.",
    );
    assert!(has(&facts, "hit", "by", "identity"), "{facts:?}");
    assert!(!has(&facts, "hit", "by", "instance"), "{facts:?}");
}

/// GH #6851 review: a document parsed at run time becomes a quoted formula, so its
/// top-level `@forAll` binds in that formula — the same term as the inline spelling.
#[test]
fn a_parsed_document_s_universal_is_its_formula_s_own() {
    let src = ":f :is { @forAll :x. :x :p :b }.
        { :f :is ?f. \"@prefix : <http://ex/>. @forAll :x. :x :p :b.\" log:parsedAsN3 ?g.
          ?f log:equalTo ?g } => { :parsed :equals :inline }.
        { :f :is ?f. \"@prefix : <http://ex/>. @forAll :x. { :x :p :b } => { :x :q :b }.\" log:parsedAsN3 ?g.
          { @forAll :x. { :x :p :b } => { :x :q :b } } log:equalTo ?g } => { :parsed :equals :inlineRule }.";
    let facts = closure(src);
    assert!(has(&facts, "parsed", "equals", "inline"), "{facts:?}");
    assert!(has(&facts, "parsed", "equals", "inlineRule"), "{facts:?}");
    // A formula fact under a DOCUMENT-level `@forAll` is a different term: its `:x` is
    // bound outside the formula.
    let facts = closure(
        "@forAll :x. :f :is { :x :p :b }.
         { :f :is ?f. \"@prefix : <http://ex/>. @forAll :x. :x :p :b.\" log:parsedAsN3 ?g.
           ?f log:notEqualTo ?g } => { :parsed :differs :fromDocumentLevel }.",
    );
    assert!(has(&facts, "parsed", "differs", "fromDocumentLevel"), "{facts:?}");
    // log:semantics parses the same way.
    let sem = format!(
        "@prefix : <{EX}>. :f :is {{ @forAll :x. :x :p :b }}.
         {{ :f :is ?f. <http://doc/d> log:semantics ?g. ?f log:equalTo ?g }} => {{ :sem :equals :inline }}."
    );
    let resolver = |iri: &str| (iri == "http://doc/d").then(|| format!("@prefix : <{EX}>. @forAll :x. :x :p :b."));
    let facts = sparq_reason::n3::reason_n3_terms_with_resolver(&sem, None, Some(&resolver as &sparq_reason::n3::Resolver))
        .expect("reasons")
        .facts;
    assert!(has(&facts, "sem", "equals", "inline"), "{facts:?}");
}

/// GH #6851 review: a parsed document bound as an aggregation clause is a quantified clause,
/// refused like the inline one.
#[test]
fn a_parsed_quantified_clause_is_rejected() {
    for rule in [
        "( :t ?c ?l ) log:collectAllIn { :a :p :b }",
        "( ?c { :a :p :b } ) log:forAllIn { :a :p :b }",
        "{ :a :p :b } log:includes ?c",
    ] {
        let src = format!(
            "@prefix : <{EX}>. :a :p :b.
             {{ \"@prefix : <http://ex/>. @forAll :x. :x :p :b.\" log:parsedAsN3 ?c. {rule} }} => {{ :hit :by :rule }}."
        );
        assert_eq!(reason_n3_terms(&src, None).err().expect("rejected"), QUANTIFIER_ERROR, "{src}");
    }
}
