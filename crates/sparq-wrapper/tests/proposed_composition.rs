// sq-kf56o: exercise all five wrapper proposals through one live graph.
#![cfg(all(
    feature = "proposed-distinct",
    feature = "proposed-cardinality",
    feature = "proposed-codecs",
    feature = "proposed-observe",
    feature = "proposed-typed-focus"
))]

use oxrdf::{NamedNode, Term};
use sparq_wrapper::proposed::cardinality::required;
use sparq_wrapper::proposed::codecs::{decode_i128, encode_i128};
use sparq_wrapper::proposed::observe::{ChangeKind, ObservableStore};
use sparq_wrapper::proposed::typed_focus::NodeFactory;
use sparq_wrapper::Store;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{local}")).expect("valid IRI")
}

#[test]
fn proposed_features_compose_over_one_live_graph_without_stale_reads() {
    let alice = iri("alice");
    let bob = iri("bob");
    let reading = iri("reading");
    let first = i128::from(i64::MAX) + 1;
    let second = first + 1;
    let mut observed = ObservableStore::new();
    let graph_address = observed.graph() as *const _;

    let seen = Rc::new(RefCell::new(Vec::new()));
    let seen_by_observer = Rc::clone(&seen);
    observed.subscribe(move |event, graph| {
        let Term::Literal(literal) = &event.object else {
            panic!("the composed reading mutation must carry a literal");
        };
        seen_by_observer.borrow_mut().push((
            event.kind,
            decode_i128(literal).expect("observer decodes the committed value"),
            graph.len(),
        ));
    });

    observed
        .live_values(alice.clone(), reading.clone())
        .insert(encode_i128(first))
        .expect("insert Alice's reading");
    observed
        .live_values(bob.clone(), reading.clone())
        .insert(encode_i128(first))
        .expect("insert Bob's equal reading");

    {
        let borrowed = Store::borrowed(observed.graph());
        let factory = NodeFactory::for_store(&borrowed);
        assert!(std::ptr::eq(factory.graph(), observed.graph()));
        assert_eq!(factory.graph() as *const _, graph_address);

        let typed_alice = factory.iri(alice.clone());
        let traversed: Vec<_> = typed_alice.out(&reading).values().collect();
        assert_eq!(traversed, vec![Term::Literal(encode_i128(first))]);

        let alice_node = typed_alice.as_node();
        let decoded = required(&alice_node, &reading, |node| {
            let Term::Literal(literal) = node.into_term() else {
                panic!("the cardinality-selected reading must be a literal");
            };
            decode_i128(&literal)
        })
        .expect("Alice has exactly one decodable reading");
        assert_eq!(decoded, first);

        let dataset = borrowed.dataset();
        let subjects: HashSet<_> = dataset.subjects_of(&reading).values().collect();
        assert_eq!(
            subjects,
            [Term::from(alice.clone()), Term::from(bob.clone())].into()
        );
        assert_eq!(
            dataset.objects_of(&reading).values().collect::<Vec<_>>(),
            vec![Term::Literal(encode_i128(first))],
            "the shared encoded object remains distinct across two subjects"
        );
    }

    {
        let mut live = observed.live_values(alice.clone(), reading.clone());
        assert!(live
            .remove(encode_i128(first))
            .expect("remove Alice's old reading"));
        assert!(live
            .insert(encode_i128(second))
            .expect("insert Alice's replacement reading"));
        assert_eq!(
            live.values(),
            vec![Term::Literal(encode_i128(second))],
            "the same live collection must re-query after mutation"
        );
    }

    let borrowed = Store::borrowed(observed.graph());
    let factory = NodeFactory::for_store(&borrowed);
    assert!(std::ptr::eq(factory.graph(), observed.graph()));
    assert_eq!(factory.graph() as *const _, graph_address);

    let typed_alice = factory.iri(alice);
    let alice_node = typed_alice.as_node();
    let decoded = required(&alice_node, &reading, |node| {
        let Term::Literal(literal) = node.into_term() else {
            panic!("the replacement reading must be a literal");
        };
        decode_i128(&literal)
    })
    .expect("the refreshed graph still has exactly one Alice reading");
    assert_eq!(decoded, second);
    assert_eq!(
        typed_alice.out(&reading).values().collect::<Vec<_>>(),
        vec![Term::Literal(encode_i128(second))]
    );

    let objects: HashSet<_> = borrowed.dataset().objects_of(&reading).values().collect();
    assert_eq!(
        objects,
        [
            Term::Literal(encode_i128(first)),
            Term::Literal(encode_i128(second))
        ]
        .into()
    );
    assert_eq!(
        *seen.borrow(),
        vec![
            (ChangeKind::Add, first, 1),
            (ChangeKind::Add, first, 2),
            (ChangeKind::Delete, first, 1),
            (ChangeKind::Add, second, 2),
        ],
        "observers must decode each mutation from its committed graph state"
    );
}
