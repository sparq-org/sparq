//! (gh-6614) Explicit input graph scopes + dataset-preserving SHACL-AF expansion.
//!
//! A capability addition (not a conformance fix): callers choose which asserted
//! graphs the rule engine sees ([`GraphScope`]), where derived triples are
//! materialized ([`Destination`]), and get back the whole dataset with every
//! asserted named graph preserved. The legacy `apply_rules` / `expand` entry
//! points are pinned unchanged here too.
#![cfg(feature = "shacl-af")]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_shacl::rules::{
    apply_rules, apply_rules_in_scope, expand, expand_dataset, Destination, GraphScope,
};
use sparq_shacl::view::GraphView;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(s).unwrap())
}

/// The issue's synthetic dataset: one default-graph triple, a Person in each of
/// g1 / g2, and an unrelated graph that must survive expansion untouched.
fn dataset() -> Graph {
    Graph::load_dataset(
        r#"
<urn:ex:root> <urn:ex:keep> "default" .
<urn:ex:a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:ex:Person> <urn:ex:g1> .
<urn:ex:b> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:ex:Person> <urn:ex:g2> .
<urn:ex:m> <urn:ex:keep> "unrelated" <urn:ex:other> .
"#,
        "nquads",
    )
    .unwrap()
}

fn triple_rule_shapes() -> Graph {
    Graph::load_str(
        r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <urn:ex:> .
ex:S a sh:NodeShape;
  sh:targetClass ex:Person;
  sh:rule [
    a sh:TripleRule;
    sh:subject sh:this;
    sh:predicate ex:derived;
    sh:object ex:Agent
  ] .
"#,
        "turtle",
    )
    .unwrap()
}

fn derived_subjects(triples: &[oxrdf::Triple]) -> Vec<String> {
    let mut v: Vec<String> = triples
        .iter()
        .filter(|t| t.predicate.as_str() == "urn:ex:derived")
        .map(|t| t.subject.to_string())
        .collect();
    v.sort();
    v
}

fn has(g: &Graph, s: &str, p: &str, o: &Term) -> bool {
    GraphView::new(g).contains(&iri(s), p, o)
}

fn len(g: &Graph) -> usize {
    GraphView::new(g).triples(None, None, None).len()
}

/// The issue's example, verbatim in intent: the legacy API sees only the default
/// graph and `expand` drops named graphs; the input is never mutated.
#[test]
fn legacy_behaviour_is_unchanged() {
    let data = dataset();
    let shapes = triple_rule_shapes();
    let g1 = iri("urn:ex:g1");
    assert!(apply_rules(&data, &shapes).triples.is_empty());
    assert_eq!(
        apply_rules(data.named_graph(&g1).unwrap(), &shapes)
            .triples
            .len(),
        1
    );
    assert!(expand(&data, &shapes).named_graph(&g1).is_none());
    assert!(data.named_graph(&g1).is_some());
    assert!(data.fork().named_graph(&g1).is_some());
}

#[test]
fn default_scope_matches_legacy() {
    let data = dataset();
    let shapes = triple_rule_shapes();
    let inf = apply_rules_in_scope(&data, &shapes, &GraphScope::Default);
    assert!(inf.triples.is_empty());
    assert_eq!(inf.triples.len(), apply_rules(&data, &shapes).triples.len());
}

#[test]
fn named_scope_derives_only_for_that_graph() {
    let data = dataset();
    let shapes = triple_rule_shapes();
    let inf = apply_rules_in_scope(&data, &shapes, &GraphScope::Named(iri("urn:ex:g1")));
    assert_eq!(derived_subjects(&inf.triples), vec!["<urn:ex:a>"]);
    // An absent graph name denotes the empty graph: nothing to derive.
    let none = apply_rules_in_scope(&data, &shapes, &GraphScope::Named(iri("urn:ex:nope")));
    assert!(none.triples.is_empty());
}

#[test]
fn union_scope_is_explicit_and_never_automatic() {
    let data = dataset();
    let shapes = triple_rule_shapes();
    let both = GraphScope::Union {
        default: false,
        named: vec![iri("urn:ex:g1"), iri("urn:ex:g2")],
    };
    let inf = apply_rules_in_scope(&data, &shapes, &both);
    assert_eq!(
        derived_subjects(&inf.triples),
        vec!["<urn:ex:a>", "<urn:ex:b>"]
    );
    // Only g2 selected: g1's Person is invisible.
    let only_g2 = GraphScope::Union {
        default: true,
        named: vec![iri("urn:ex:g2")],
    };
    let inf = apply_rules_in_scope(&data, &shapes, &only_g2);
    assert_eq!(derived_subjects(&inf.triples), vec!["<urn:ex:b>"]);
    // An empty union sees nothing at all.
    let empty = GraphScope::Union {
        default: false,
        named: vec![],
    };
    assert!(apply_rules_in_scope(&data, &shapes, &empty)
        .triples
        .is_empty());
}

/// Targets in one graph, path values in another: a union scope joins them for
/// targets, `sh:condition` and the rule's path expression alike.
#[test]
fn union_scope_joins_targets_paths_and_conditions_across_graphs() {
    let data = Graph::load_dataset(
        r#"
<urn:ex:a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:ex:Person> <urn:ex:profiles> .
<urn:ex:b> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:ex:Person> <urn:ex:profiles> .
<urn:ex:a> <urn:ex:email> "a@ex" <urn:ex:observations> .
<urn:ex:a> <urn:ex:verified> "true"^^<http://www.w3.org/2001/XMLSchema#boolean> <urn:ex:provenance> .
<urn:ex:b> <urn:ex:email> "b@ex" <urn:ex:observations> .
"#,
        "nquads",
    )
    .unwrap();
    let shapes = Graph::load_str(
        r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <urn:ex:> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:S a sh:NodeShape;
  sh:targetClass ex:Person;
  sh:rule [
    a sh:TripleRule;
    sh:subject sh:this;
    sh:predicate ex:contact;
    sh:object [ sh:path ex:email ];
    sh:condition ex:Verified
  ] .
ex:Verified a sh:NodeShape;
  sh:property [ sh:path ex:verified; sh:hasValue true ] .
"#,
        "turtle",
    )
    .unwrap();
    let all = GraphScope::Union {
        default: false,
        named: vec![
            iri("urn:ex:profiles"),
            iri("urn:ex:observations"),
            iri("urn:ex:provenance"),
        ],
    };
    let inf = apply_rules_in_scope(&data, &shapes, &all);
    assert_eq!(inf.triples.len(), 1, "{:?}", inf.triples);
    assert_eq!(inf.triples[0].subject.to_string(), "<urn:ex:a>");
    assert_eq!(inf.triples[0].object.to_string(), "\"a@ex\"");
    // Without the provenance graph the condition fails for everyone.
    let no_prov = GraphScope::Union {
        default: false,
        named: vec![iri("urn:ex:profiles"), iri("urn:ex:observations")],
    };
    assert!(apply_rules_in_scope(&data, &shapes, &no_prov)
        .triples
        .is_empty());
}

fn sparql_graph_rule_shapes(graph_pattern: &str) -> Graph {
    Graph::load_str(
        &format!(
            r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <urn:ex:> .
ex:S a sh:NodeShape;
  sh:targetClass ex:Person;
  sh:rule [
    a sh:SPARQLRule;
    sh:construct """CONSTRUCT {{ $this <urn:ex:seenIn> ?g }} WHERE {{ {graph_pattern} }}"""
  ] .
"#
        ),
        "turtle",
    )
    .unwrap()
}

/// SPARQLRule `GRAPH ?g` / `GRAPH <g>` see exactly the selected named graphs;
/// unselected graphs (and FROM NAMED) cannot widen that boundary.
#[test]
fn sparql_rule_graph_access_is_limited_to_the_selected_graphs() {
    let data = dataset();
    let scope = GraphScope::Union {
        default: false,
        named: vec![iri("urn:ex:g1"), iri("urn:ex:g2")],
    };

    let var = sparql_graph_rule_shapes("GRAPH ?g { $this a <urn:ex:Person> }");
    let inf = apply_rules_in_scope(&data, &var, &scope);
    let mut got: Vec<String> = inf
        .triples
        .iter()
        .map(|t| format!("{} {}", t.subject, t.object))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec!["<urn:ex:a> <urn:ex:g1>", "<urn:ex:b> <urn:ex:g2>"]
    );

    // A disallowed graph (`other`, not selected) matches nothing, even though the
    // input dataset holds it.
    let other = sparql_graph_rule_shapes(
        "GRAPH <urn:ex:other> { ?m <urn:ex:keep> ?v } BIND(<urn:ex:other> AS ?g)",
    );
    assert!(apply_rules_in_scope(&data, &other, &scope)
        .triples
        .is_empty());
    // The legacy API exposes no named graphs to rule bodies at all.
    assert!(apply_rules(&data, &var).triples.is_empty());
    // Named(g1) exposes g1 (and only g1) as a named graph too.
    let inf = apply_rules_in_scope(&data, &var, &GraphScope::Named(iri("urn:ex:g1")));
    assert_eq!(inf.triples.len(), 1);
    assert_eq!(inf.triples[0].object.to_string(), "<urn:ex:g1>");
}

#[test]
fn sparql_rule_dataset_clause_cannot_bypass_the_scope() {
    let data = dataset();
    let shapes = Graph::load_str(
        r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <urn:ex:> .
ex:S a sh:NodeShape;
  sh:targetClass ex:Person;
  sh:rule [
    a sh:SPARQLRule;
    sh:construct """CONSTRUCT { $this <urn:ex:leak> ?v } FROM NAMED <urn:ex:other> WHERE { GRAPH <urn:ex:other> { ?m <urn:ex:keep> ?v } }"""
  ] .
"#,
        "turtle",
    )
    .unwrap();
    let scope = GraphScope::Named(iri("urn:ex:g1"));
    assert!(apply_rules_in_scope(&data, &shapes, &scope)
        .triples
        .is_empty());
}

#[test]
fn expand_dataset_preserves_every_asserted_graph_and_the_input() {
    let data = dataset();
    let shapes = triple_rule_shapes();
    let scope = GraphScope::Union {
        default: false,
        named: vec![iri("urn:ex:g1"), iri("urn:ex:g2")],
    };
    let out = expand_dataset(&data, &shapes, &scope, &Destination::Default);
    assert_eq!(out.inference.triples.len(), 2);
    assert!(!out.inference.capped);
    // Default graph = asserted default + derived.
    assert!(has(
        &out.dataset,
        "urn:ex:root",
        "urn:ex:keep",
        &Term::Literal("default".into())
    ));
    assert!(has(
        &out.dataset,
        "urn:ex:a",
        "urn:ex:derived",
        &iri("urn:ex:Agent")
    ));
    assert!(has(
        &out.dataset,
        "urn:ex:b",
        "urn:ex:derived",
        &iri("urn:ex:Agent")
    ));
    assert_eq!(len(&out.dataset), 3);
    // Every asserted named graph — including the unrelated one — survives as-is.
    for (name, n) in [("urn:ex:g1", 1), ("urn:ex:g2", 1), ("urn:ex:other", 1)] {
        let g = out.dataset.named_graph(&iri(name)).expect(name);
        assert_eq!(len(g), n, "{name}");
    }
    assert!(has(
        out.dataset.named_graph(&iri("urn:ex:g1")).unwrap(),
        "urn:ex:a",
        RDF_TYPE,
        &iri("urn:ex:Person")
    ));
    // Input not mutated.
    assert_eq!(len(&data), 1);
    assert_eq!(data.named.len(), 3);
}

#[test]
fn expand_dataset_into_a_named_destination() {
    let data = dataset();
    let shapes = triple_rule_shapes();
    let scope = GraphScope::Named(iri("urn:ex:g1"));
    let dest = Destination::Named(iri("urn:ex:derived"));
    let out = expand_dataset(&data, &shapes, &scope, &dest);
    // Default graph untouched.
    assert_eq!(len(&out.dataset), 1);
    let d = out.dataset.named_graph(&iri("urn:ex:derived")).unwrap();
    assert_eq!(len(d), 1);
    assert!(has(d, "urn:ex:a", "urn:ex:derived", &iri("urn:ex:Agent")));
    assert_eq!(out.dataset.named.len(), 4);
    // Materializing into an EXISTING named graph merges and dedups there, while
    // an identical assertion in another graph is not collapsed.
    let into_g1 = expand_dataset(
        &data,
        &shapes,
        &scope,
        &Destination::Named(iri("urn:ex:g1")),
    );
    let g1 = into_g1.dataset.named_graph(&iri("urn:ex:g1")).unwrap();
    assert_eq!(len(g1), 2);
    assert_eq!(into_g1.dataset.named.len(), 3);
}

/// A derivation already asserted in the destination is not duplicated, while the
/// same triple asserted in another graph stays in both.
#[test]
fn materialization_dedups_per_destination() {
    let data = Graph::load_dataset(
        r#"
<urn:ex:a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:ex:Person> <urn:ex:g1> .
<urn:ex:a> <urn:ex:derived> <urn:ex:Agent> <urn:ex:g1> .
<urn:ex:a> <urn:ex:derived> <urn:ex:Agent> .
"#,
        "nquads",
    )
    .unwrap();
    let shapes = triple_rule_shapes();
    let scope = GraphScope::Named(iri("urn:ex:g1"));
    let out = expand_dataset(&data, &shapes, &scope, &Destination::Default);
    assert_eq!(len(&out.dataset), 1);
    assert_eq!(len(out.dataset.named_graph(&iri("urn:ex:g1")).unwrap()), 2);
    let out = expand_dataset(
        &data,
        &shapes,
        &scope,
        &Destination::Named(iri("urn:ex:g1")),
    );
    assert_eq!(len(&out.dataset), 1);
    assert_eq!(len(out.dataset.named_graph(&iri("urn:ex:g1")).unwrap()), 2);
}

/// Inferred triples feed later passes inside the working scope regardless of the
/// persisted destination; the fixpoint diagnostics are reported.
#[test]
fn fixpoint_chains_independently_of_destination() {
    let data = Graph::load_dataset(
        r#"
<urn:ex:a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:ex:Person> <urn:ex:g1> .
"#,
        "nquads",
    )
    .unwrap();
    let shapes = Graph::load_str(
        r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <urn:ex:> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
ex:S1 a sh:NodeShape; sh:targetClass ex:Person;
  sh:rule [ a sh:TripleRule; sh:subject sh:this; sh:predicate rdf:type; sh:object ex:Agent ] .
ex:S2 a sh:NodeShape; sh:targetClass ex:Agent;
  sh:rule [ a sh:TripleRule; sh:subject sh:this; sh:predicate ex:active; sh:object true ] .
"#,
        "turtle",
    )
    .unwrap();
    let out = expand_dataset(
        &data,
        &shapes,
        &GraphScope::Named(iri("urn:ex:g1")),
        &Destination::Named(iri("urn:ex:inferred")),
    );
    assert_eq!(out.inference.triples.len(), 2);
    assert!(out.inference.iterations >= 2);
    assert!(!out.inference.capped);
    assert_eq!(
        len(out.dataset.named_graph(&iri("urn:ex:inferred")).unwrap()),
        2
    );
}

#[test]
fn cap_diagnostic_propagates_through_expand_dataset() {
    let data = Graph::load_dataset(
        "<urn:ex:a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:ex:Person> <urn:ex:g1> .\n",
        "nquads",
    )
    .unwrap();
    // A CONSTRUCT minting a fresh blank node each pass never reaches a fixpoint.
    let shapes = Graph::load_str(
        r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <urn:ex:> .
ex:S a sh:NodeShape; sh:targetClass ex:Person;
  sh:rule [ a sh:SPARQLRule;
    sh:construct "CONSTRUCT { $this <urn:ex:p> _:b } WHERE { $this a <urn:ex:Person> }" ] .
"#,
        "turtle",
    )
    .unwrap();
    let out = expand_dataset(
        &data,
        &shapes,
        &GraphScope::Named(iri("urn:ex:g1")),
        &Destination::Default,
    );
    assert!(out.inference.capped);
    assert_eq!(out.inference.iterations, sparq_shacl::rules::MAX_ITERATIONS);
}
