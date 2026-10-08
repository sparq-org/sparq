// sq-1rg2q.9: async mapped cardinality reads and live mapped sets, observed
// through the shared poll-counting fake backend so the assertions see how much of the
// stream the wrapper actually pulled.

#![cfg(feature = "proposed-async-node")]

mod common;

use common::{block_on, delayed_store, iri, term, FakeStore, Traversal};
use oxrdf::{Literal, NamedNode, Term};
use sparq_wrapper::proposed::async_node::{live_set, many, optional, required, AsyncMapError};
use sparq_wrapper::proposed::async_store::{AsyncNode, AsyncStore, AsyncStoreError};
use sparq_wrapper::{Cardinality, CardinalityError};
use std::cell::Cell;

fn literal(value: &str) -> Term {
    Literal::new_simple_literal(value).into()
}

#[test]
fn singular_reads_stop_polling_after_the_second_value() {
    for exactly_one in [true, false] {
        let (state, store) = delayed_store(0);
        let alice = store.node(iri("alice"));
        let knows = iri("knows");
        let mapped = Cell::new(false);
        let map = |_| {
            mapped.set(true);
            Ok::<_, ()>(())
        };

        let error = if exactly_one {
            block_on(required(&alice, &knows, map)).unwrap_err()
        } else {
            block_on(optional(&alice, &knows, map)).unwrap_err()
        };

        assert_eq!(
            error,
            AsyncMapError::Cardinality(CardinalityError {
                focus: term("alice"),
                predicate: knows,
                expected: if exactly_one {
                    Cardinality::ExactlyOne
                } else {
                    Cardinality::AtMostOne
                },
                found: 2,
            })
        );
        // NON-VACUOUS: the fake holds three matches; draining would emit 3 and poll 4.
        assert_eq!(state.emitted.get(), 2);
        assert_eq!(state.stream_polls.get(), 2);
        assert!(!state.drained.get());
        assert_eq!(state.traversal(0), Traversal::Cancelled);
        assert!(
            !mapped.get(),
            "a cardinality violation never reaches the mapper"
        );
    }
}

#[test]
fn singular_reads_map_one_value_and_report_empty_results() {
    let (_, store) = delayed_store(1);
    let alice = store.node(iri("alice"));
    let name = iri("name");
    block_on(alice.add(name.clone(), literal("Alice")).unwrap()).unwrap();

    let display = block_on(required(&alice, &name, |node| match node.into_term() {
        Term::Literal(value) => Ok::<_, ()>(value.value().to_owned()),
        _ => Err(()),
    }));
    assert_eq!(display, Ok("Alice".to_owned()));

    let missing = iri("missing");
    let never = |_| -> Result<(), ()> { panic!("the empty case never maps") };
    assert_eq!(block_on(optional(&alice, &missing, never)), Ok(None));
    assert!(matches!(
        block_on(required(&alice, &missing, never)),
        Err(AsyncMapError::Cardinality(CardinalityError {
            found: 0,
            ..
        }))
    ));
}

#[test]
fn mapped_values_chain_async_out_and_in_traversal() {
    let (state, store) = delayed_store(1);
    let alice = store.node(iri("alice"));
    let knows = iri("knows");

    let friends = block_on(many(&alice, &knows, |node| Ok::<_, ()>(node.into_term()))).unwrap();
    assert_eq!(friends, vec![term("bob"), term("carol"), term("dave")]);
    assert!(state.drained.get());

    // A mapped node stays bound to the store, so it can traverse back in.
    let bob = block_on(many(&alice, &knows, Ok::<_, ()>))
        .unwrap()
        .remove(0);
    let mut incoming = bob.r#in(&knows);
    let source = block_on(incoming.next()).unwrap().unwrap();
    assert_eq!(source.into_term(), term("alice"));
}

#[test]
fn live_set_mutations_write_through_to_the_backing_store() {
    let (state, store) = delayed_store(1);
    let tag = iri("tag");
    let tags = live_set(
        &store,
        iri("alice"),
        tag.clone(),
        |node: AsyncNode<'_, FakeStore>| match node.into_term() {
            Term::Literal(value) => Ok(value.value().to_owned()),
            other => Err(format!("not a literal: {other}")),
        },
        |value: &String| {
            if value.is_empty() {
                Err("empty tag".to_owned())
            } else {
                Ok(literal(value))
            }
        },
    );
    let rdf = "rdf".to_owned();

    assert_eq!(block_on(tags.insert(&rdf)), Ok(true));
    assert_eq!(block_on(tags.insert(&rdf)), Ok(false));
    assert!(state.contains(&term("alice"), &tag, &literal("rdf")));
    assert_eq!(block_on(tags.contains(&rdf)), Ok(true));
    assert_eq!(block_on(tags.values()), Ok(vec![rdf.clone()]));

    // The written triple is visible to incoming traversal from the object side.
    let mut subjects = store.node(literal("rdf")).r#in(&tag);
    assert_eq!(
        block_on(subjects.next()).unwrap().unwrap().into_term(),
        term("alice")
    );

    let before = state.triples.borrow().len();
    assert_eq!(
        block_on(tags.insert(&String::new())),
        Err(AsyncMapError::Conversion("empty tag".to_owned()))
    );
    assert_eq!(state.triples.borrow().len(), before);

    assert_eq!(block_on(tags.remove(&rdf)), Ok(true));
    assert_eq!(block_on(tags.remove(&rdf)), Ok(false));
    assert!(!state.contains(&term("alice"), &tag, &literal("rdf")));
    assert_eq!(block_on(tags.values()), Ok(Vec::new()));
}

#[test]
fn backend_and_subject_errors_surface_as_store_errors() {
    let (state, store) = delayed_store(0);
    state.fail_reads.set(true);
    let alice = store.node(iri("alice"));
    assert_eq!(
        block_on(required(&alice, &iri("knows"), |node| Ok::<_, ()>(
            node.into_term()
        ))),
        Err(AsyncMapError::Store(AsyncStoreError::Backend(
            "link down".to_owned()
        )))
    );

    let empty: AsyncStore<FakeStore> = delayed_store(0).1;
    let set = live_set(
        &empty,
        literal("not a subject"),
        NamedNode::new("http://example.org/p").unwrap(),
        |_: AsyncNode<'_, FakeStore>| Ok::<(), ()>(()),
        |_: &()| Ok::<_, ()>(literal("o")),
    );
    assert_eq!(
        block_on(set.insert(&())),
        Err(AsyncMapError::Store(AsyncStoreError::LiteralSubject))
    );
    assert_eq!(
        AsyncMapError::<String>::Store(AsyncStoreError::LiteralSubject).to_string(),
        "RDF literals cannot be triple subjects"
    );
}

#[test]
fn live_set_reports_only_the_change_its_own_write_made() {
    use std::future::Future;
    use std::pin::pin;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::task::{Context, Waker};

    let (state, store) = delayed_store(2);
    let other = AsyncStore::new(FakeStore {
        state: Rc::clone(&state),
        delay: 0,
    });
    let tag = iri("tag");
    let tags = live_set(
        &store,
        iri("alice"),
        tag.clone(),
        |node: AsyncNode<'_, FakeStore>| Ok::<_, ()>(node.into_term()),
        |value: &Term| Ok::<_, ()>(value.clone()),
    );
    let waker = Waker::from(Arc::new(common::TestWaker::default()));
    let mut cx = Context::from_waker(&waker);

    for inserting in [true, false] {
        let rdf = literal("rdf");
        let mut pending = pin!(async {
            if inserting {
                tags.insert(&rdf).await
            } else {
                tags.remove(&rdf).await
            }
        });
        // Suspend inside the live set's own write, then let another client of
        // the same backend make the identical change first.
        let issued = state.writes_issued.get();
        while state.writes_issued.get() == issued {
            assert!(pending.as_mut().poll(&mut cx).is_pending());
        }
        let raced = if inserting {
            other.add(iri("alice"), tag.clone(), rdf.clone())
        } else {
            other.delete(iri("alice"), tag.clone(), rdf.clone())
        };
        assert_eq!(block_on(raced.unwrap()), Ok(true));
        assert_eq!(block_on(pending), Ok(false), "inserting: {inserting}");
    }
}
