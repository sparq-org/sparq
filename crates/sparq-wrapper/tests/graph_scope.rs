// [GPT-5.6] sq-1rg2q.6: graph-scope projection and write-target witnesses.

#![cfg(feature = "proposed-graph-scope")]

use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_wrapper::proposed::graph_scope::{GraphScope, GraphScopeError};
use sparq_wrapper::Store;

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{local}")).unwrap()
}

fn graph_term(local: &str) -> Term {
    Term::NamedNode(iri(local))
}

fn objects(graph: &Graph, subject: &NamedNode, predicate: &NamedNode) -> Vec<Term> {
    Store::borrowed(graph)
        .node(subject.clone())
        .out(predicate)
        .values()
        .collect()
}

#[test]
fn reads_exact_named_scope_deduplicates_and_node_writes_only_to_target() {
    let mut graph = Graph::load_dataset(
        "<http://example.org/alice> <http://example.org/tag> \"shared\" <http://example.org/g1> .\n\
         <http://example.org/alice> <http://example.org/tag> \"shared\" <http://example.org/g2> .\n\
         <http://example.org/alice> <http://example.org/tag> \"excluded\" <http://example.org/g3> .\n",
        "nquads",
    )
    .unwrap();
    let g1 = graph_term("g1");
    let g2 = graph_term("g2");
    let g3 = graph_term("g3");
    let alice = iri("alice");
    let tag = iri("tag");

    {
        let scope = GraphScope::new(&mut graph, [g1.clone(), g2.clone()], g1.clone());
        let alice_node = scope.node(alice.clone());

        // NON-VACUOUS: changing this expected singleton or its term makes the
        // acceptance test fail. The duplicate is projected once and g3 is absent.
        assert_eq!(
            alice_node.out(&tag).values().collect::<Vec<_>>(),
            vec![Term::Literal(Literal::new_simple_literal("shared"))]
        );

        alice_node
            .insert(tag.clone(), Literal::new_simple_literal("new"))
            .unwrap();
        let values = alice_node.out(&tag).values().collect::<Vec<_>>();
        assert_eq!(values.len(), 2);
        assert!(values.contains(&Term::Literal(Literal::new_simple_literal("new"))));
        assert!(values.contains(&Term::Literal(Literal::new_simple_literal("shared"))));
    }

    let new_value = Term::Literal(Literal::new_simple_literal("new"));
    assert!(objects(graph.named_graph(&g1).unwrap(), &alice, &tag).contains(&new_value));
    assert!(!objects(graph.named_graph(&g2).unwrap(), &alice, &tag).contains(&new_value));
    assert!(!objects(graph.named_graph(&g3).unwrap(), &alice, &tag).contains(&new_value));
    assert!(!objects(&graph, &alice, &tag).contains(&new_value));
}

#[test]
fn remove_leaves_copies_in_every_other_graph_untouched() {
    let mut graph = Graph::load_dataset(
        "<http://example.org/alice> <http://example.org/tag> \"shared\" .\n\
         <http://example.org/alice> <http://example.org/tag> \"shared\" <http://example.org/g1> .\n\
         <http://example.org/alice> <http://example.org/tag> \"shared\" <http://example.org/g2> .\n",
        "nquads",
    )
    .unwrap();
    let g1 = graph_term("g1");
    let g2 = graph_term("g2");
    let alice = iri("alice");
    let tag = iri("tag");
    let shared = Term::Literal(Literal::new_simple_literal("shared"));

    {
        let scope = GraphScope::new(&mut graph, [g1.clone(), g2.clone()], g1.clone());
        scope
            .node(alice.clone())
            .remove(tag.clone(), shared.clone())
            .unwrap();
    }

    assert!(!objects(graph.named_graph(&g1).unwrap(), &alice, &tag).contains(&shared));
    assert!(objects(graph.named_graph(&g2).unwrap(), &alice, &tag).contains(&shared));
    assert!(objects(&graph, &alice, &tag).contains(&shared));
}

#[test]
fn default_graph_is_read_only_when_explicitly_configured() {
    let mut graph = Graph::load_dataset(
        "<http://example.org/alice> <http://example.org/tag> \"shared\" .\n\
         <http://example.org/alice> <http://example.org/tag> \"default-only\" .\n\
         <http://example.org/alice> <http://example.org/tag> \"shared\" <http://example.org/g1> .\n",
        "nquads",
    )
    .unwrap();
    let alice = iri("alice");
    let tag = iri("tag");
    let g1 = graph_term("g1");

    let without_default = GraphScope::new(&mut graph, [g1.clone()], g1.clone());
    assert_eq!(
        without_default
            .node(alice.clone())
            .out(&tag)
            .values()
            .collect::<Vec<_>>(),
        vec![Term::Literal(Literal::new_simple_literal("shared"))]
    );
    drop(without_default);

    let with_default = GraphScope::new(&mut graph, [g1.clone()], g1).with_default_graph();
    let values = with_default
        .node(alice)
        .out(&tag)
        .values()
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 2);
    assert!(values.contains(&Term::Literal(Literal::new_simple_literal("shared"))));
    assert!(values.contains(&Term::Literal(Literal::new_simple_literal("default-only"))));
}

#[test]
fn scoped_incoming_traversal_deduplicates_and_missing_write_graph_is_created() {
    let mut graph = Graph::load_dataset(
        "<http://example.org/alice> <http://example.org/knows> <http://example.org/bob> <http://example.org/g1> .\n\
         <http://example.org/alice> <http://example.org/knows> <http://example.org/bob> <http://example.org/g2> .\n",
        "nquads",
    )
    .unwrap();
    let write = graph_term("write");

    {
        let mut scope = GraphScope::new(
            &mut graph,
            [graph_term("g1"), graph_term("g2")],
            write.clone(),
        );
        assert_eq!(
            scope
                .node(iri("bob"))
                .r#in(&iri("knows"))
                .values()
                .collect::<Vec<_>>(),
            vec![Term::NamedNode(iri("alice"))]
        );
        scope
            .insert(iri("bob"), iri("knows"), iri("carol"))
            .unwrap();
        assert_eq!(
            scope.remove(
                Literal::new_simple_literal("not a subject"),
                iri("knows"),
                iri("carol")
            ),
            Err(GraphScopeError::LiteralSubject)
        );
    }

    assert_eq!(
        objects(
            graph.named_graph(&write).unwrap(),
            &iri("bob"),
            &iri("knows")
        ),
        vec![Term::NamedNode(iri("carol"))]
    );
}

#[test]
fn literal_write_graph_is_rejected_before_a_graph_is_created() {
    let mut graph = Graph::new();
    {
        let mut scope = GraphScope::new(
            &mut graph,
            [graph_term("g1")],
            Literal::new_simple_literal("g"),
        );
        let rejected = Err(GraphScopeError::InvalidGraphName(
            Literal::new_simple_literal("g").into(),
        ));
        assert_eq!(
            scope.insert(iri("alice"), iri("knows"), iri("bob")),
            rejected
        );
        assert_eq!(
            scope.remove(iri("alice"), iri("knows"), iri("bob")),
            rejected
        );
    }
    assert!(graph.named.is_empty());
}
