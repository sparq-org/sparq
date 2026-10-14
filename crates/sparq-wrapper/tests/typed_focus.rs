// sq-1rg2q.2: typed focus kinds + bound node factories (rdfjs/wrapper #83-#87).
#![cfg(feature = "proposed-typed-focus")]

use oxrdf::{BlankNode, Literal, NamedNode, Term, Triple};
use sparq_core::Graph;
use sparq_wrapper::proposed::typed_focus::{AnyNode, NodeFactory};
use sparq_wrapper::{Store, TermKind};

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{local}")).expect("valid IRI")
}

/// alice --knows--> bob --knows--> carol; alice --name--> "Alice";
/// _:home --occupant--> alice.
fn sample_graph() -> Graph {
    let mut graph = Graph::new();
    let knows = iri("knows");
    graph
        .insert_triple(iri("alice"), knows.clone(), iri("bob"))
        .expect("insert");
    graph
        .insert_triple(iri("bob"), knows, iri("carol"))
        .expect("insert");
    graph
        .insert_triple(
            iri("alice"),
            iri("name"),
            Literal::new_simple_literal("Alice"),
        )
        .expect("insert");
    graph
        .insert_triple(home(), iri("occupant"), iri("alice"))
        .expect("insert");
    graph
}

fn home() -> BlankNode {
    BlankNode::new("home").expect("valid blank node id")
}

// ACCEPTANCE: one bound factory wraps multiple terms over the same borrowed
// store and traverses them. (The compile-fail half of the acceptance test —
// a literal focus cannot be supplied as a subject — is the `compile_fail`
// doctest on `NodeFactory`.)
#[test]
fn one_bound_factory_wraps_many_terms_over_one_borrowed_store_and_traverses() {
    let graph = sample_graph();
    let store = Store::borrowed(&graph);
    let factory = NodeFactory::for_store(&store);

    // IRI focus: outgoing and incoming traversal.
    let alice = factory.iri(iri("alice"));
    let friends: Vec<Term> = alice.out(&iri("knows")).values().collect();
    assert_eq!(friends, vec![Term::NamedNode(iri("bob"))]);

    // The SAME factory (it is `Copy`) wraps a second subject...
    let bob = factory.iri(iri("bob"));
    let friends_of_friends: Vec<Term> = bob.out(&iri("knows")).values().collect();
    assert_eq!(friends_of_friends, vec![Term::NamedNode(iri("carol"))]);
    let knowers: Vec<Term> = bob.r#in(&iri("knows")).values().collect();
    assert_eq!(knowers, vec![Term::NamedNode(iri("alice"))]);

    // ...a literal (object-position traversal only)...
    let name = factory.literal(Literal::new_simple_literal("Alice"));
    let named: Vec<Term> = name.r#in(&iri("name")).values().collect();
    assert_eq!(named, vec![Term::NamedNode(iri("alice"))]);

    // ...and a blank node (subject-position traversal).
    let house = factory.blank(home());
    let occupants: Vec<Term> = house.out(&iri("occupant")).values().collect();
    assert_eq!(occupants, vec![Term::NamedNode(iri("alice"))]);
}

#[test]
fn factory_binds_the_borrowed_store_without_cloning_it() {
    let graph = sample_graph();
    let store = Store::borrowed(&graph);
    let factory = NodeFactory::for_store(&store);

    // The factory is one shared reference to the store's graph — the same
    // allocation, not a clone or a materialized view.
    assert!(std::ptr::eq(factory.graph(), store.graph()));
    assert!(std::ptr::eq(factory.graph(), &graph));

    // Every wrapped node shares that same graph reference too.
    let node = factory.iri(iri("alice"));
    assert!(std::ptr::eq(node.dataset().graph(), &graph));

    // And the dataset-bound constructor binds the identical reference.
    let from_dataset = NodeFactory::for_dataset(store.dataset());
    assert!(std::ptr::eq(from_dataset.graph(), &graph));
}

#[test]
fn term_factory_preserves_the_concrete_kind() {
    let graph = sample_graph();
    let factory = NodeFactory::new(&graph);
    let triple = Triple::new(iri("alice"), iri("knows"), iri("bob"));

    let wrapped = factory.term(Term::NamedNode(iri("alice")));
    assert!(matches!(wrapped, AnyNode::Iri(_)));
    assert_eq!(wrapped.kind(), TermKind::NamedNode);
    assert_eq!(wrapped.term(), Term::NamedNode(iri("alice")));

    let wrapped = factory.term(Term::BlankNode(home()));
    assert!(matches!(wrapped, AnyNode::Blank(_)));
    assert_eq!(wrapped.kind(), TermKind::BlankNode);

    let wrapped = factory.term(Term::Literal(Literal::new_simple_literal("Alice")));
    assert!(matches!(wrapped, AnyNode::Literal(_)));
    assert_eq!(wrapped.kind(), TermKind::Literal);

    let wrapped = factory.term(Term::from(triple.clone()));
    assert!(matches!(wrapped, AnyNode::Triple(_)));
    assert_eq!(wrapped.kind(), TermKind::Triple);
    assert_eq!(wrapped.term(), Term::from(triple));
}

#[test]
fn any_node_kind_recovers_position_legal_traversal() {
    let graph = sample_graph();
    let factory = NodeFactory::new(&graph);

    // Matching an `AnyNode` variant recovers a focus kind on which the
    // position-legal operations compile.
    match factory.term(Term::NamedNode(iri("alice"))) {
        AnyNode::Iri(alice) => {
            let friends: Vec<Term> = alice.out(&iri("knows")).values().collect();
            assert_eq!(friends, vec![Term::NamedNode(iri("bob"))]);
        }
        other => panic!("expected an IRI focus, got {other:?}"),
    }
}

#[test]
fn predicate_focus_scans_subject_and_object_positions() {
    let graph = sample_graph();
    let factory = NodeFactory::new(&graph);
    let knows = factory.iri(iri("knows"));

    // Scan order is an index detail; compare as sets (duplicates impossible
    // here because the sample graph has no repeated subject/object per role).
    let subjects: std::collections::HashSet<Term> = knows.subjects().values().collect();
    let expected: std::collections::HashSet<Term> =
        [Term::NamedNode(iri("alice")), Term::NamedNode(iri("bob"))].into();
    assert_eq!(subjects, expected);

    let objects: std::collections::HashSet<Term> = knows.objects().values().collect();
    let expected: std::collections::HashSet<Term> =
        [Term::NamedNode(iri("bob")), Term::NamedNode(iri("carol"))].into();
    assert_eq!(objects, expected);
}

#[test]
fn absent_focus_terms_traverse_empty() {
    let graph = sample_graph();
    let factory = NodeFactory::new(&graph);

    assert_eq!(factory.iri(iri("nobody")).out(&iri("knows")).len(), 0);
    assert_eq!(
        factory
            .literal(Literal::new_simple_literal("nobody"))
            .r#in(&iri("name"))
            .len(),
        0
    );
    assert_eq!(factory.iri(iri("unused-predicate")).subjects().len(), 0);
    let absent_triple = factory.triple(Triple::new(iri("x"), iri("y"), iri("z")));
    assert_eq!(absent_triple.out(&iri("knows")).len(), 0);
    assert_eq!(absent_triple.r#in(&iri("knows")).len(), 0);
}

#[test]
fn debug_formats_name_the_kind_without_dumping_the_graph() {
    let graph = sample_graph();
    let factory = NodeFactory::new(&graph);

    let rendered = format!("{:?}", factory.iri(iri("alice")));
    assert!(rendered.contains("TypedNode"), "got {rendered}");
    assert!(rendered.contains("NamedNode"), "got {rendered}");
    assert!(rendered.contains("alice"), "got {rendered}");

    let rendered = format!("{factory:?}");
    assert!(rendered.contains("NodeFactory"), "got {rendered}");
}

#[test]
fn typed_nodes_erase_to_the_untyped_wrapper() {
    let graph = sample_graph();
    let factory = NodeFactory::new(&graph);

    let typed = factory.literal(Literal::new_simple_literal("Alice"));
    assert_eq!(typed.kind(), TermKind::Literal);
    assert_eq!(
        typed.focus(),
        &Literal::new_simple_literal("Alice"),
        "typed focus accessor returns the concrete literal"
    );

    // The erased node still supports the untyped typed-value accessors.
    let node = typed.as_node();
    assert_eq!(node.as_str().expect("simple literal"), "Alice");
    assert_eq!(
        typed.clone().into_node().focus(),
        &Term::Literal(Literal::new_simple_literal("Alice"))
    );
    assert_eq!(
        typed.into_focus(),
        Literal::new_simple_literal("Alice"),
        "into_focus unwraps the concrete term"
    );

    let any = factory.term(Term::NamedNode(iri("alice")));
    let friends: Vec<Term> = any.into_node().out(&iri("knows")).values().collect();
    assert_eq!(friends, vec![Term::NamedNode(iri("bob"))]);
}
