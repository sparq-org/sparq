//! GH #6754: a formula-level `@forAll` binds only in the formula that declares it.
//!
//! The parser names such a universal by its declared IRI plus its de Bruijn distance to
//! the declaring formula (`?__uf.<n>.<iri>`), so the name is unforgeable (a `.` cannot
//! occur in a source quickvar) and canonical (alpha-equivalent copies of a formula are the
//! same term). A rule premise's own universal is proved only by an independent universal
//! of the store (GH #6882), and refused where it cannot be matched soundly; every other
//! formula-level universal belongs to its quoted formula, matches only itself and is never
//! substituted. Document-level `@forAll` is unchanged.

use sparq_reason::n3::{parser, Term};
use sparq_reason::{reason_n3_pass_all, reason_n3_terms, RuleVars};

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

/// Rules whose premise declares its own universal where no engine matches it soundly: a
/// builtin operand, a quoted formula, a variable predicate, a list relation, a backward
/// rule, or a document with backward rules (GH #6882 supports the rest).
fn premise_cases() -> Vec<String> {
    [
        "{ @forAll :x. ( 1 2 ) math:sum :x } => { :hit :by :rule }.",
        "@forAll :z. :z :p :b. { @forAll :x. :x :p :b. :x log:notEqualTo :c } => { :hit :by :rule }.",
        // In a quoted formula of a stored-predicate atom.
        "@forAll :z. :a :says { :z :p :b }. { @forAll :x. :a :says { :x :p :b } } => { :hit :by :rule }.",
        // A variable predicate and the list relations evaluate list terms too.
        "@forAll :z. :z :p :b. { @forAll :x. :x ?p :b } => { :hit :by :rule }.",
        "{ @forAll :x. ( :x ) rdf:first :x } => { :hit :by :rule }.",
        // Backward rules have the same unsupported antecedent.
        "{ :hit :by :rule } <= { @forAll :x. :x :p :b }.",
        // A backward proof would read a store universal in a goal as a variable.
        "@forAll :z. :z :p :b. { @forAll :x. :x :p :b } => { :hit :by :rule }. { :c :q :d } <= { :c :p :b }.",
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
fn naive_rejects_unsupported_premise_universals() {
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
fn compiled_rejects_unsupported_premise_universals() {
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
fn incremental_rejects_unsupported_premise_universals() {
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
    // A query premise is matched with no firing check: refused.
    let src = format!("@prefix : <{EX}>. {{ @forAll :x. :x :p :b }} => {{ :hit :by :rule }}.");
    assert_eq!(sparq_reason::reason_n3_query_terms("", &src).unwrap_err(), QUANTIFIER_ERROR);
}

// GH #6882: a rule premise's own `@forAll` (`{ @forAll :x. :x :p :b } => …`) reads
// `∀x. x :p :b`. Only a universal of the store proves it, and only when nothing else the
// firing binds depends on that universal. Each case records what EYE (eyereasoner npm
// 21.1.25) derives where it differs.

/// Does the premise-universal rule of `src` (concluding `:hit :by :rule`) fire?
fn hits(src: &str) -> bool {
    has(&closure(src), "hit", "by", "rule")
}

#[test]
fn a_ground_fact_does_not_prove_a_premise_universal() {
    // `:c :p :b` is one instance, not `∀x. x :p :b` (EYE agrees).
    assert!(!hits(":c :p :b. { @forAll :x. :x :p :b } => { :hit :by :rule }."));
    assert!(!hits(":c :p :b. { @forAll :x. :x :p :b } => { @forAll :x. :x :q :b. :hit :by :rule }."));
}

#[test]
fn a_stored_universal_proves_a_premise_universal() {
    assert!(hits("@forAll :y. :y :p :b. { @forAll :x. :x :p :b } => { :hit :by :rule }."));
    // Through two atoms on the same store universal.
    assert!(hits("@forAll :z. :z :p :b. :z :q :r. { @forAll :x. :x :p :b. :x :q :r } => { :hit :by :rule }."));
    // Inside a list and a quoted triple of the stored term.
    assert!(hits("@forAll :z. :a :q ( :z :c ). { @forAll :x. :a :q ( :x :c ) } => { :hit :by :rule }."));
    assert!(hits("@forAll :z. :a :q <<( :z :r :s )>>. { @forAll :x. :a :q <<( :x :r :s )>> } => { :hit :by :rule }."));
    // The universal never reaches the conclusion: a quickvar spelled like it is unbound.
    let facts = closure("@forAll :z. :z :p :b. { @forAll :x. :x :p :b } => { ?__up_x :q :b. ?__uf0_x :q :b. :hit :by :rule }.");
    assert!(has(&facts, "hit", "by", "rule"), "{facts:?}");
    assert!(!facts.iter().any(|f| f[1] == iri("q")), "{facts:?}");
}

/// The W3C N3 test `tests/N3Tests/quantification/simple_forAll_premise.n3` (in no
/// manifest), verbatim.
#[test]
fn w3c_simple_for_all_premise() {
    let src = "@prefix : <http://example.org#>.\n\n@prefix : <http://example.org#>.\n\n\
               @forAll :x. :x :p :o.\n\n{@forAll :y. :y :p :o }=>{:test :was :successful}. \n";
    let facts = reason_n3_terms(src, None).expect("reasons").facts;
    let ex = |l: &str| Term::Iri(format!("http://example.org#{l}"));
    assert!(facts.contains(&[ex("test"), ex("was"), ex("successful")]), "{facts:?}");
}

#[test]
fn an_ordinary_variable_must_not_alias_the_store_universal() {
    // `∀z. z :p z` does not give `∃y ∀x. x :p y`. (EYE fires on both.)
    assert!(!hits("@forAll :z. :z :p :z. { @forAll :x. :x :p ?y } => { :hit :by :rule }."));
    // The same with the ordinary variable matched first.
    assert!(!hits("@forAll :z. :z :p :z. :z :q :z. { @forAll :x. ?y :q ?y. :x :p ?y } => { :hit :by :rule }."));
    // A different store universal is independent: `∀z ∀w. z :p w` gives `∀x. x :p w`.
    let facts = closure("@forAll :z, :w. :z :p :w. { @forAll :x. :x :p ?v } => { :hit :by :rule. :hit :with { :v :is ?v } }.");
    assert!(has(&facts, "hit", "by", "rule"), "{facts:?}");
}

#[test]
fn an_ordinary_variable_must_not_carry_the_store_universal_in_a_value() {
    // ?l binds a list, a quoted formula or a quoted triple that mentions `:z`.
    for value in ["( :z )", "{ :z :r :s }", "<<( :z :r :s )>>"] {
        let src = format!("@forAll :z. :z :p :b. :a :q {value}. {{ @forAll :x. :x :p :b. :a :q ?l }} => {{ :hit :by :rule }}.");
        assert!(!hits(&src), "{src}");
        // With the ordinary variable matched first.
        let src = format!("@forAll :z. :z :p :b. :a :q {value}. {{ @forAll :x. :a :q ?l. :x :p :b }} => {{ :hit :by :rule }}.");
        assert!(!hits(&src), "{src}");
    }
    // A value with no store universal in it is fine.
    assert!(hits("@forAll :z. :z :p :b. :a :q ( :c ). { @forAll :x. :x :p :b. :a :q ?l } => { :hit :by :rule }."));
}

#[test]
fn an_ordinary_variable_must_not_bind_an_existential_witness() {
    // `∀z ∃b. z :p b` does not give `∃y ∀x. x :p y`: the witness may depend on `:z`.
    assert!(!hits("@forAll :z. :z :p _:b. { @forAll :x. :x :p ?y } => { :hit :by :rule }."));
    assert!(!hits("@forAll :z. :z :p [ :q :r ]. { @forAll :x. :x :p ?y } => { :hit :by :rule }."));
    // A blank anywhere in the value, matched in either order.
    for value in ["( _:b )", "{ _:b :r :s }", "<<( _:b :r :s )>>"] {
        let src = format!("@forAll :z. :z :p {value}. {{ @forAll :x. :x :p ?y }} => {{ :hit :by :rule }}.");
        assert!(!hits(&src), "{src}");
        let src = format!("@forAll :z. :z :p {value}. :z :q :c. {{ @forAll :x. :x :p ?y. :x :q :c }} => {{ :hit :by :rule }}.");
        assert!(!hits(&src), "{src}");
    }
    // A premise blank is a variable too: it may not bind the witness either.
    assert!(!hits("@forAll :z. :z :p _:b. { @forAll :x. :x :p _:y } => { :hit :by :rule }."));
}

#[test]
fn distinct_premise_universals_need_distinct_store_universals() {
    // `∀z. z :p z` does not give `∀x ∀y. x :p y` (EYE agrees)...
    assert!(!hits("@forAll :z. :z :p :z. { @forAll :x, :y. :x :p :y } => { :hit :by :rule }."));
    // ...but does give `∀x. x :p x`, and `∀z ∀w. z :p w` gives `∀x ∀y. x :p y`.
    assert!(hits("@forAll :z. :z :p :z. { @forAll :x. :x :p :x } => { :hit :by :rule }."));
    assert!(hits("@forAll :z, :w. :z :p :w. { @forAll :x, :y. :x :p :y } => { :hit :by :rule }."));
}

#[test]
fn a_transitivity_shaped_rule_still_checks_its_premise_universal() {
    // `{ ?x :P ?y. ?y :P ?z } => { ?x :P ?z }` has a fast path; with `?y` the premise's own
    // universal the rule is `∀x ∀z. (∀y. x :P y ∧ y :P z) → x :P z`.
    let rule = "{ @forAll :y. ?x :P :y. :y :P ?z } => { ?x :P ?z }.";
    let facts = closure(&format!(":a :P :b. :b :P :c. {rule}"));
    assert!(!has(&facts, "a", "P", "c"), "{facts:?}");
    let facts = closure(&format!("@forAll :u. :a :P :u. :u :P :c. {rule}"));
    assert!(has(&facts, "a", "P", "c"), "{facts:?}");
}

#[test]
fn an_ordinary_variable_still_binds_no_universal_fact_into_a_conclusion() {
    // Unchanged: `?s` binds the universal, and a conclusion left with a variable at the
    // triple level is not asserted.
    let facts = closure("@forAll :x. :x :p :o. { ?s :p :o } => { :hit :by ?s }.");
    assert!(!facts.iter().any(|f| f[0] == iri("hit")), "{facts:?}");
}

/// The closure-run cases: `:k`'s formula is closed by `log:conclusion`, and the outer
/// rule reports whether the inner premise-universal rule fired there.
fn fires_in_closure(formula: &str) -> bool {
    let src = format!(
        ":k :is {formula}.\n\
         {{ :k :is ?f. ?f log:conclusion ?g. ?g log:includes {{ :hit :by :rule }} }} => {{ :closure :fired :yes }}.\n"
    );
    has(&closure(&src), "closure", "fired", "yes")
}

#[test]
fn a_closure_run_s_own_universal_proves_a_premise_universal() {
    assert!(fires_in_closure("{ @forAll :x. :x :p :b. { @forAll :y. :y :p :b } => { :hit :by :rule } }"));
    assert!(!fires_in_closure("{ :c :p :b. { @forAll :y. :y :p :b } => { :hit :by :rule } }"));
    assert!(!fires_in_closure("{ @forAll :x. :x :p :x. { @forAll :y. :y :p ?v } => { :hit :by :rule } }"));
}

#[test]
fn a_closure_rule_s_premise_blank_does_not_match_a_dependent_witness() {
    // In a closed formula a rule's premise blank is a constant naming the formula's own
    // existential, whose witness depends on the universal (`∀y ∃w. y :p w`).
    for formula in [
        "{ @forAll :y. @forSome :w. :y :p :w. { @forAll :x. :x :p :w } => { :hit :by :rule } }",
        "{ @forAll :y. :y :p _:b. { @forAll :x. :x :p _:b } => { :hit :by :rule } }",
        "{ @forAll :y. :y :p ( _:b ). { @forAll :x. :x :p ( _:b ) } => { :hit :by :rule } }",
    ] {
        let src = format!(
            ":k :is {formula}.\n\
             {{ :k :is ?f. ?f log:conclusion ?g. ?g log:includes {{ :hit :by :rule }} }} => {{ :closure :fired :yes }}.\n"
        );
        match reason_n3_terms(&format!("@prefix : <{EX}>.\n{src}"), None) {
            Ok(r) => assert!(!has(&r.facts, "closure", "fired", "yes"), "{src}"),
            Err(e) => assert!(e.contains("universally quantified antecedents"), "{e}"),
        }
    }
}

#[test]
fn an_outer_universal_does_not_prove_a_closure_s_premise_universal() {
    // The document's `:z` is a free parameter of the closed formula.
    let src = "@forAll :z. :k :is { :z :p :b. { @forAll :y. :y :p :b } => { :hit :by :rule } }.\n\
               { :k :is ?f. ?f log:conclusion ?g. ?g log:includes { :hit :by :rule } } => { :closure :fired :yes }.\n";
    assert!(!has(&closure(src), "closure", "fired", "yes"));
    // An enclosing formula's universal, in a closure nested in another closure's run.
    let src = ":k :is { @forAll :x. :x :p :b.\n\
                 :k2 :is { :x :p :b. { @forAll :y. :y :p :b } => { :hit :by :rule } }.\n\
                 { :k2 :is ?f. ?f log:conclusion ?g. ?g log:includes { :hit :by :rule } } => { :inner :fired :yes } }.\n\
               { :k :is ?f. ?f log:conclusion ?g. ?g log:includes { :inner :fired :yes } } => { :closure :fired :yes }.\n";
    assert!(!has(&closure(src), "closure", "fired", "yes"));
    // The same nesting with the inner formula's own universal does fire.
    let src = ":k :is { :c :p :b.\n\
                 :k2 :is { @forAll :x. :x :p :b. { @forAll :y. :y :p :b } => { :hit :by :rule } }.\n\
                 { :k2 :is ?f. ?f log:conclusion ?g. ?g log:includes { :hit :by :rule } } => { :inner :fired :yes } }.\n\
               { :k :is ?f. ?f log:conclusion ?g. ?g log:includes { :inner :fired :yes } } => { :closure :fired :yes }.\n";
    assert!(has(&closure(src), "closure", "fired", "yes"));
}

/// The W3C `nested_universal*.n3` premises quote a RULE (`{ @forAll :x. { :x :p :o } =>
/// { … } }`), which matches only a rule stored as a fact; this engine keeps a document's
/// rules out of its facts, and a premise universal inside a quoted formula is refused.
#[test]
fn a_premise_universal_in_a_quoted_rule_is_refused() {
    let src = "@prefix : <http://example.org#>.\n{?x :p :o}=>{?x :p2 :o2}.\n\
               {@forAll :x. {:x :p :o}=>{:x :p2 :o2}  }=>{:test :was :successful}.";
    assert_eq!(reason_n3_terms(src, None).err().expect("rejected"), QUANTIFIER_ERROR);
}

#[cfg(feature = "compiled-rules")]
#[test]
fn compiled_premise_universals_never_fire_on_ground_ids() {
    // A compiled store holds ground ids only, so the rule compiles and never fires, as the
    // text engine never fires it over ground facts. A document universal is refused.
    use sparq_reason::n3::compiled::{compile, eval, intern_facts};
    let rules = format!("@prefix : <{EX}>. {{ @forAll :x. :x :p :b }} => {{ :hit :by :rule }}. {{ :c :p :b }} => {{ :plain :rule :fires }}.");
    let compiled = compile(&rules).expect("compiles");
    let mut dict = sparq_core::dict::Dict::new();
    let facts = intern_facts(&mut dict, &format!("@prefix : <{EX}>. :c :p :b.")).expect("facts");
    let closure = eval(&mut dict, &facts, &compiled).expect("closure");
    assert_eq!(closure.len(), 2, "the fact and the plain rule's derivation only");
    let text = reason_n3_terms(&format!("{rules} :c :p :b."), None).expect("reasons").facts;
    assert!(!has(&text, "hit", "by", "rule") && has(&text, "plain", "rule", "fires"), "{text:?}");
    let stored = format!("@prefix : <{EX}>. @forAll :y. :y :p :b. {{ @forAll :x. :x :p :b }} => {{ :hit :by :rule }}.");
    assert!(compile(&stored).is_err());
}

#[cfg(feature = "explain")]
#[test]
fn a_premise_universal_firing_is_explained_by_the_stored_universal() {
    let src = format!("@prefix : <{EX}>. @forAll :y. :y :p :b. {{ @forAll :x. :x :p :b }} => {{ :hit :by :rule }}.");
    let proof = sparq_reason::why_n3_document(&src, &[iri("hit"), iri("by"), iri("rule")], sparq_reason::ExplainOpts::default())
        .expect("parses")
        .expect("derived");
    let nodes = proof.nodes();
    let root = nodes.last().expect("root");
    assert_eq!(root.premises.len(), 1, "{nodes:?}");
    let premise = &nodes[root.premises[0] as usize];
    assert_eq!(premise.rule, "asserted");
    assert!(premise.conclusion[1].contains("http://ex/p"), "{premise:?}");
}

#[test]
fn incremental_premise_universals_match_the_batch_engine() {
    // The counting path leaves such a rule to the batch engine, which fires it.
    let src = format!("@prefix : <{EX}>. @forAll :y. :y :p :b. {{ @forAll :x. :x :p :b }} => {{ :hit :by :rule }}.");
    let g = sparq_reason::MaterializedN3Graph::new(&src, &[]).expect("supported");
    assert!(g.fallback_reason().is_some_and(|r| r.contains("premise-local @forAll")), "{:?}", g.fallback_reason());
    assert!(g.contains(&[iri("hit"), iri("by"), iri("rule")]));
    // A ground fact inserted later does not prove it.
    let src = format!("@prefix : <{EX}>. {{ @forAll :x. :x :p :b }} => {{ :hit :by :rule }}.");
    let mut g = sparq_reason::MaterializedN3Graph::new(&src, &[]).expect("supported");
    assert_eq!(g.insert(&[[iri("c"), iri("p"), iri("b")]]), 1);
    assert!(g.contains(&[iri("c"), iri("p"), iri("b")]));
    assert!(!g.contains(&[iri("hit"), iri("by"), iri("rule")]));
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

#[test]
fn a_bound_quantified_containment_pattern_is_rejected() {
    for src in bound_pattern_cases("{ @forAll :x. :x :p :b }") {
        assert_eq!(reason_n3_terms(&src, None).err().expect("rejected"), QUANTIFIER_ERROR, "{src}");
        let base = parser::parse(&src).expect("parses").facts;
        let err = sparq_reason::MaterializedN3Graph::new(&src, &base).err().expect("rejected");
        assert!(err.starts_with(QUANTIFIER_ERROR), "{src}: {err}");
    }
}

/// A bound pattern without a formula-level universal is not refused.
#[test]
fn a_bound_unquantified_containment_pattern_is_not_refused() {
    for src in bound_pattern_cases("{ :c :p :b }") {
        assert!(reason_n3_terms(&src, None).is_ok(), "{src}");
        let base = parser::parse(&src).expect("parses").facts;
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
