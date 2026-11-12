//! Fixture-independent behavioral guards for the SHACL 1.2 rule execution model.
#![cfg(feature = "shacl-af")]
use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_shacl::{rules12, view::GraphView};
const PREFIX: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> . @prefix ex: <urn:ex:> . @prefix shnex: <http://www.w3.org/ns/shacl-node-expr#> . @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .";
fn graph(s: &str) -> Graph {
    Graph::load_str(&format!("{PREFIX}\n{s}"), "turtle").unwrap()
}
fn run(data: &str, shapes: &str) -> Vec<oxrdf::Triple> {
    rules12::apply_rules(&graph(data), &graph(shapes), None)
        .unwrap()
        .triples
}
fn has(triples: &[oxrdf::Triple], s: &str, p: &str, o: &str) -> bool {
    triples.iter().any(|t| {
        t.subject.to_string() == format!("<urn:ex:{s}>")
            && t.predicate.as_str() == format!("urn:ex:{p}")
            && t.object.to_string() == format!("<urn:ex:{o}>")
    })
}
#[test]
fn global_rules_and_base_subtraction() {
    let out=run("ex:a ex:p ex:b . ex:b ex:p ex:a .", "ex:R a sh:SPARQLRule ; sh:construct 'CONSTRUCT {?y <urn:ex:p> ?x} WHERE {?x <urn:ex:p> ?y}' .");
    assert!(out.is_empty());
    let out=run("ex:a ex:p ex:b .", "ex:R a sh:SPARQLRule ; sh:construct 'CONSTRUCT {?y <urn:ex:p> ?x} WHERE {?x <urn:ex:p> ?y}' .");
    assert_eq!(out.len(), 1);
    assert!(has(&out, "b", "p", "a"));
}
#[test]
fn lower_order_visible_but_same_order_uses_snapshot() {
    let shapes="ex:R1 a sh:SPARQLRule ; sh:order 1 ; sh:construct 'CONSTRUCT {<urn:ex:a> <urn:ex:p> <urn:ex:b>} WHERE {}' . ex:R2 a sh:SPARQLRule ; sh:order 2 ; sh:construct 'CONSTRUCT {<urn:ex:a> <urn:ex:q> <urn:ex:b>} WHERE {FILTER NOT EXISTS {<urn:ex:a> <urn:ex:p> <urn:ex:b>}}' .";
    let ordered = run("", shapes);
    assert_eq!(ordered.len(), 1);
    let same = run("", &shapes.replace("sh:order 2", "sh:order 1"));
    assert_eq!(same.len(), 2);
    assert!(has(&same, "a", "q", "b"));
}
#[test]
fn layer_fixpoint_precedes_next_layer() {
    let out=run("ex:a ex:p ex:b . ex:b ex:p ex:c . ex:c ex:p ex:d .", "ex:R1 a sh:SPARQLRule ; sh:construct 'CONSTRUCT {?a <urn:ex:p> ?c} WHERE {?a <urn:ex:p> ?b . ?b <urn:ex:p> ?c}' . ex:R2 a sh:SPARQLRule ; sh:layer 1 ; sh:construct 'CONSTRUCT {<urn:ex:a> <urn:ex:q> <urn:ex:d>} WHERE {FILTER NOT EXISTS {<urn:ex:a> <urn:ex:p> <urn:ex:d>}}' .");
    assert!(has(&out, "a", "p", "d"));
    assert!(!has(&out, "a", "q", "d"));
    assert_eq!(out.len(), 3);
}
#[test]
fn run_once_shared_targets_and_deactivated_shape() {
    let shapes="ex:S1 sh:targetNode ex:a ; sh:rule ex:R . ex:S2 sh:targetNode ex:a ; sh:rule ex:R . ex:S3 sh:targetNode ex:b ; sh:rule ex:R ; sh:deactivated true . ex:R a sh:SPARQLRule ; sh:runOnce true ; sh:construct 'CONSTRUCT {$this <urn:ex:head> []} WHERE {}' .";
    let out = run("", shapes);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].subject.to_string(), "<urn:ex:a>");
    assert!(matches!(out[0].object, Term::BlankNode(_)));
}
#[test]
fn blank_focus_nested_limit_is_prebound() {
    let out=run("_:a a ex:T . _:b a ex:T .", "ex:S sh:targetClass ex:T ; sh:rule ex:R . ex:R a sh:SPARQLRule ; sh:construct 'CONSTRUCT {$this <urn:ex:ok> true} WHERE {{SELECT $this WHERE {} LIMIT 1}}' .");
    assert_eq!(out.len(), 2);
    assert_ne!(out[0].subject, out[1].subject);
    assert!(out
        .iter()
        .all(|t| matches!(t.subject, oxrdf::NamedOrBlankNode::BlankNode(_))));
}
#[test]
fn omitted_triple_operands_default_to_focus_and_global_empty() {
    let out = run(
        "",
        "ex:S sh:targetNode ex:a ; sh:rule ex:R . ex:R a sh:TripleRule ; sh:predicate ex:p .",
    );
    assert_eq!(out.len(), 1);
    assert!(has(&out, "a", "p", "a"));
    assert!(run(
        "",
        "ex:R a sh:TripleRule ; sh:predicate ex:p ; sh:object ex:b ."
    )
    .is_empty());
}
#[test]
fn expression_failure_is_not_an_empty_success() {
    let s=graph("ex:S sh:targetNode ex:a ; sh:rule [a sh:TripleRule ; sh:predicate ex:p ; sh:object [shnex:count [shnex:pathValues ex:q ; shnex:focusNode (ex:a ex:b)]]] .");
    let error = rules12::apply_rules(&graph(""), &s, None).unwrap_err();
    assert!(
        error.contains("Path expression requires one focus node"),
        "{error}"
    );
}
#[test]
fn template_optional_and_required_parameters() {
    let s="ex:Template a sh:SPARQLRuleTemplate ; rdfs:subClassOf sh:Rule ; sh:parameter [sh:path <urn:arg/p>] ; sh:parameter [sh:path <urn:arg/optional> ; sh:optional true] ; sh:construct 'CONSTRUCT {<urn:ex:a> $p ?v} WHERE {BIND(COALESCE($optional,<urn:ex:b>) AS ?v)}' . ex:R a ex:Template ; <urn:arg/p> ex:p .";
    let out = run("", s);
    assert_eq!(out.len(), 1);
    assert!(has(&out, "a", "p", "b"));
    assert!(rules12::apply_rules(
        &graph(""),
        &graph(&s.replace("; <urn:arg/p> ex:p", "")),
        None
    )
    .is_err());
}
#[test]
fn cyclic_rule_sets_select_only_members_and_reject_processors() {
    let s=graph("ex:A a sh:RuleSet ; sh:includesRuleSet ex:B . ex:B a sh:RuleSet ; sh:includesRuleSet ex:A ; sh:hasRule ex:R . ex:R a sh:TripleRule ; sh:subject ex:a ; sh:predicate ex:p ; sh:object ex:b . ex:Other a sh:TripleRule ; sh:subject ex:x ; sh:predicate ex:p ; sh:object ex:y .");
    let set: Term = NamedNode::new_unchecked("urn:ex:A").into();
    let out = rules12::apply_rules(&graph(""), &s, Some(&set))
        .unwrap()
        .triples;
    assert_eq!(out.len(), 1);
    assert!(has(&out, "a", "p", "b"));
    let bad=graph("ex:A a sh:RuleSet ; sh:includesRuleSet ex:B . ex:B a sh:RuleSet ; sh:ruleProcessor 'unknown' .");
    assert!(rules12::apply_rules(&graph(""), &bad, Some(&set)).is_err());
}
#[test]
fn derived_values_frozen_per_layer_and_kept_only_if_inferred() {
    let s="ex:S sh:targetNode ex:a ; sh:property [sh:path ex:computed ; sh:values [shnex:count [shnex:pathValues ex:p]]] . ex:R1 a sh:TripleRule ; sh:subject ex:a ; sh:predicate ex:p ; sh:object ex:b . ex:R2 a sh:SPARQLRule ; sh:expectedPredicate ex:computed ; sh:construct 'CONSTRUCT {<urn:ex:a> <urn:ex:n> ?n} WHERE {<urn:ex:a> <urn:ex:computed> ?n}' .";
    let out = run("", s);
    assert_eq!(out.len(), 2);
    assert!(out
        .iter()
        .any(|t| t.predicate.as_str() == "urn:ex:n" && t.object == oxrdf::Literal::from(0).into()));
    assert!(!out
        .iter()
        .any(|t| t.predicate.as_str() == "urn:ex:computed"));
    let s=format!("{s} ex:R3 a sh:SPARQLRule ; sh:expectedPredicate ex:computed ; sh:construct 'CONSTRUCT {{<urn:ex:a> <urn:ex:computed> ?n}} WHERE {{<urn:ex:a> <urn:ex:computed> ?n}}' .");
    assert!(run("", &s)
        .iter()
        .any(|t| t.predicate.as_str() == "urn:ex:computed"));
}
#[test]
fn temporary_triples_are_visible_then_removed() {
    let out=run("", "ex:R1 a sh:SPARQLRule ; sh:runOnce true ; sh:construct 'CONSTRUCT {<urn:ex:a> <urn:ex:p> <urn:ex:b> {| <http://www.w3.org/ns/shacl#tempTriple> true |}} WHERE {}' . ex:R2 a sh:SPARQLRule ; sh:layer 1 ; sh:construct 'CONSTRUCT {<urn:ex:a> <urn:ex:q> <urn:ex:b>} WHERE {<urn:ex:a> <urn:ex:p> <urn:ex:b>}' .");
    assert_eq!(out.len(), 1);
    assert!(has(&out, "a", "q", "b"));
}
#[test]
fn precise_decimal_ordering() {
    let out=run("", "ex:R1 a sh:TripleRule ; sh:layer 9007199254740992.1 ; sh:subject ex:a ; sh:predicate ex:p ; sh:object ex:b . ex:R2 a sh:SPARQLRule ; sh:layer 9007199254740992.2 ; sh:construct 'CONSTRUCT {<urn:ex:a> <urn:ex:q> <urn:ex:b>} WHERE {FILTER NOT EXISTS {<urn:ex:a> <urn:ex:p> <urn:ex:b>}}' .");
    assert_eq!(out.len(), 1);
    assert!(has(&out, "a", "p", "b"));
}
#[test]
fn entailment_changes_validation_and_propagates_failures() {
    let data = graph("");
    let s=graph("ex:G sh:entailment sh:RulesEntailment . ex:S sh:targetNode ex:a ; sh:property [sh:path ex:p ; sh:minCount 1] . ex:R a sh:TripleRule ; sh:subject ex:a ; sh:predicate ex:p ; sh:object ex:b .");
    assert!(sparq_shacl::validate_strict(&data, &s).unwrap().conforms);
    assert!(GraphView::new(&data).triples(None, None, None).is_empty());
    let bad = graph("ex:G sh:entailment sh:RulesEntailment . ex:R a sh:Rule .");
    assert!(sparq_shacl::validate_strict(&data, &bad).is_err());
    assert!(sparq_shacl::validate(&data, &bad)
        .diagnostics
        .iter()
        .any(|d| d.source_component.ends_with("#failure")));
}
#[test]
fn conditions_include_superclasses_and_processing_failures() {
    let s="ex:Parent a sh:ShapeClass ; sh:property [sh:path ex:required ; sh:minCount 1] . ex:Child a sh:ShapeClass ; rdfs:subClassOf ex:Parent . ex:S sh:targetNode ex:a,ex:b ; sh:rule [a sh:TripleRule ; sh:condition ex:Child ; sh:predicate ex:p ; sh:object ex:ok] .";
    let out = run("ex:a ex:required true .", s);
    assert_eq!(out.len(), 1);
    assert!(has(&out, "a", "p", "ok"));
    let bad=graph("ex:S sh:targetNode ex:a ; sh:rule [a sh:TripleRule ; sh:condition [sh:sparql [sh:select 'SELECT $this ?failure WHERE {BIND(true AS ?failure)}']] ; sh:predicate ex:p ; sh:object ex:ok] .");
    assert!(rules12::apply_rules(&graph(""), &bad, None).is_err());
}
#[test]
fn syntax_runtime_and_iteration_failures_are_errors() {
    for query in [
        "not sparql",
        "CONSTRUCT {?s ?p ?o} WHERE {SERVICE <urn:missing> {?s ?p ?o}}",
    ] {
        let s = graph(&format!("ex:R a sh:SPARQLRule ; sh:construct '{query}' ."));
        assert!(rules12::apply_rules(&graph(""), &s, None).is_err());
    }
    let s = graph("ex:R a sh:SPARQLRule ; sh:construct 'CONSTRUCT {[] <urn:p> true} WHERE {}' .");
    assert!(rules12::apply_rules(&graph(""), &s, None).is_err());
}

fn processor_failure(shapes: &str, owner: &str) {
    let shapes = graph(&format!(
        "ex:Selected a sh:RuleSet ; sh:hasRule ex:Good .
         ex:Good a sh:TripleRule ; sh:subject ex:a ; sh:predicate ex:p ; sh:object ex:b .
         {shapes}"
    ));
    let selected = NamedNode::new_unchecked("urn:ex:Selected").into();
    for set in [None, Some(&selected)] {
        let error = rules12::apply_rules(&graph(""), &shapes, set).unwrap_err();
        assert_eq!(
            error,
            format!("Unsupported rule processor on <urn:ex:{owner}>")
        );
    }
}

#[test]
fn unknown_processor_on_a_rule_fails_before_selection_or_deactivation() {
    for declaration in [
        "ex:Bad a sh:TripleRule .",
        "ex:Bad a sh:SPARQLRule ; sh:deactivated true .",
        "ex:Template a sh:SPARQLRuleTemplate . ex:Bad a ex:Template .",
        "ex:Shape sh:rule ex:Bad .",
        "ex:Set sh:hasRule ex:Bad .",
    ] {
        processor_failure(
            &format!("{declaration} ex:Bad sh:ruleProcessor 'unsupported' ."),
            "Bad",
        );
    }
}

#[test]
fn unknown_processor_on_an_unselected_rule_set_fails() {
    processor_failure(
        "ex:Bad a sh:RuleSet ; sh:ruleProcessor 'unsupported' ; sh:hasRule ex:Good .",
        "Bad",
    );
    // Empty and subclassed sets are still encountered in the shapes graph.
    processor_failure(
        "ex:CustomSet rdfs:subClassOf sh:RuleSet .
         ex:Bad a ex:CustomSet ; sh:ruleProcessor ex:Unsupported .",
        "Bad",
    );
}

#[test]
fn unknown_processor_on_an_included_rule_set_fails() {
    processor_failure(
        "ex:Selected sh:includesRuleSet ex:Middle .
         ex:Middle a sh:RuleSet ; sh:includesRuleSet ex:Bad .
         ex:Bad a sh:RuleSet ; sh:includesRuleSet ex:Selected ; sh:ruleProcessor 'unsupported' .",
        "Bad",
    );
    processor_failure(
        "ex:Unselected a sh:RuleSet ; sh:includesRuleSet ex:Bad .
         ex:Bad sh:ruleProcessor 'unsupported' .",
        "Bad",
    );
    let shapes = graph(
        "ex:G sh:entailment ex:Selected .
         ex:Selected a sh:RuleSet ; sh:includesRuleSet ex:Bad .
         ex:Bad a sh:RuleSet ; sh:ruleProcessor 'unsupported' .",
    );
    let failure = sparq_shacl::validate_strict(&graph(""), &shapes).unwrap_err();
    assert!(failure.to_string().contains("Unsupported rule processor"));
}

#[test]
fn absent_processor_runs_default_selected_and_included_rules() {
    let shapes = graph(
        "ex:Selected a sh:RuleSet ; sh:includesRuleSet ex:Included .
         ex:Included a sh:RuleSet ; sh:hasRule ex:Good .
         ex:Good a sh:TripleRule ; sh:subject ex:a ; sh:predicate ex:p ; sh:object ex:b .
         ex:Annotation sh:ruleProcessor 'unrelated annotation' .",
    );
    let selected = NamedNode::new_unchecked("urn:ex:Selected").into();
    for set in [None, Some(&selected)] {
        let out = rules12::apply_rules(&graph(""), &shapes, set)
            .unwrap()
            .triples;
        assert_eq!(out.len(), 1);
        assert!(has(&out, "a", "p", "b"));
    }
}

#[test]
fn rule_queries_share_now_and_keep_literal_text() {
    let out=run("", "ex:R1 a sh:SPARQLRule ; sh:runOnce true ; sh:construct 'CONSTRUCT {<urn:ex:a> <urn:ex:p> ?t . <urn:ex:a> <urn:ex:text> \"NOW()\"} WHERE {BIND(NOW() AS ?t)}' . ex:R2 a sh:SPARQLRule ; sh:layer 1 ; sh:runOnce true ; sh:construct 'CONSTRUCT {<urn:ex:a> <urn:ex:q> ?t} WHERE {BIND(NOW() AS ?t)}' .");
    assert_eq!(out.len(), 3);
    let get = |p: &str| {
        out.iter()
            .find(|t| t.predicate.as_str() == format!("urn:ex:{p}"))
            .unwrap()
            .object
            .clone()
    };
    assert_eq!(get("p"), get("q"));
    assert_eq!(
        get("text"),
        oxrdf::Literal::new_simple_literal("NOW()").into()
    );
}

#[test]
fn invalid_rule_syntax_is_rejected() {
    for s in [
        "ex:R a sh:TripleRule ; sh:subject ex:a ; sh:predicate ex:p, ex:q ; sh:object ex:b .",
        "ex:R a sh:TripleRule ; sh:subject ex:a ; sh:predicate ex:p ; sh:object ex:b ; sh:runOnce 'yes' .",
        "ex:R a sh:SPARQLRule ; sh:construct 'CONSTRUCT {} WHERE {}', 'CONSTRUCT {<urn:s> <urn:p> <urn:o>} WHERE {}' .",
    ] { assert!(rules12::apply_rules(&graph(""),&graph(s),None).is_err()); }
}

#[test]
fn nested_property_values_are_the_derived_focus_nodes() {
    let out=run("ex:a ex:child ex:b .", "ex:S sh:targetNode ex:a ; sh:property [sh:path ex:child ; sh:property [sh:path ex:derived ; sh:defaultValue ex:value]] . ex:R a sh:SPARQLRule ; sh:expectedPredicate ex:derived ; sh:construct 'CONSTRUCT {?s <urn:ex:p> ?v} WHERE {?s <urn:ex:derived> ?v}' .");
    assert_eq!(out.len(), 1);
    assert!(has(&out, "b", "p", "value"));
    let out = run(
        "ex:a sh:shape ex:S .",
        "ex:S sh:rule [a sh:TripleRule ; sh:predicate ex:p ; sh:object ex:value] .",
    );
    assert_eq!(out.len(), 1);
    assert!(has(&out, "a", "p", "value"));
}

#[test]
fn empty_focus_and_expression_errors_follow_the_pinned_draft() {
    assert!(run("", "ex:R a sh:TripleRule ; sh:subject [shnex:pathValues ex:p] ; sh:predicate ex:q ; sh:object ex:b .").is_empty());
    for object in [
        "[shnex:instancesOf 42]",
        "[<http://www.w3.org/ns/sparql#multiply> ( [] (1 2) )]",
        "[<http://www.w3.org/ns/sparql#multiply> ( 'bad' 2 )]",
    ] {
        let shapes = graph(&format!(
            "ex:R a sh:TripleRule ; sh:subject ex:a ; sh:predicate ex:p ; sh:object {object} ."
        ));
        assert!(rules12::apply_rules(&graph(""), &shapes, None).is_err());
    }
    let bad=graph("ex:S sh:targetNode ex:a ; sh:rule [a sh:TripleRule ; sh:predicate ex:p ; sh:object ex:b ; sh:condition [sh:sparql [sh:select 'SELECT $this WHERE {MINUS {?s ?p ?o}}']]] .");
    assert!(rules12::apply_rules(&graph(""), &bad, None).is_err());
}
