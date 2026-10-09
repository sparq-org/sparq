// [SONNET-4.6] sq-1rg2q.8: streaming, cancellation, and round-trip witnesses for the
// asynchronous store wrapper. The fake backend is deliberately delayed and instrumented so
// the assertions observe WHEN the producer ran, not just what it produced.

#![cfg(feature = "proposed-async-store")]

mod common;

use common::{block_on, delayed_store, iri, poll_once, term, FakeState, FakeStore, Traversal};
use oxrdf::{Literal, Term};
use sparq_wrapper::proposed::async_store::{AsyncStore, AsyncStoreError, TermStream};
use std::pin::Pin;
use std::rc::Rc;
use std::task::Poll;

// ---------------------------------------------------------------------------
// Acceptance: streaming, cancellation, and async round-trips.
// ---------------------------------------------------------------------------

#[test]
fn first_wrapped_node_arrives_before_the_producer_finishes() {
    let (state, store) = delayed_store(1);
    let alice = store.node(iri("alice"));
    let knows = iri("knows");

    let mut stream = alice.out(&knows);
    // Constructing a traversal must not touch the backend at all.
    assert_eq!(state.stream_polls.get(), 0);
    assert_eq!(state.emitted.get(), 0);

    assert!(
        poll_once(|cx| stream.poll_next(cx)).is_pending(),
        "a delayed backend must surface as Pending, not as a blocking wait"
    );
    assert_eq!(state.emitted.get(), 0);

    let first = match poll_once(|cx| stream.poll_next(cx)) {
        Poll::Ready(Some(Ok(node))) => node,
        other => panic!("expected the first wrapped node, got {:?}", other),
    };

    assert_eq!(first.focus(), &term("bob"));
    // The witness: exactly one term has been produced and the stream has not
    // reached its end, so the wrapper handed a node back mid-production.
    assert_eq!(state.emitted.get(), 1);
    assert!(!state.drained.get());
}

#[test]
fn dropping_the_stream_stops_further_polls() {
    let (state, store) = delayed_store(1);
    let alice = store.node(iri("alice"));
    let knows = iri("knows");

    let mut stream = alice.out(&knows);
    assert!(poll_once(|cx| stream.poll_next(cx)).is_pending());
    assert!(matches!(
        poll_once(|cx| stream.poll_next(cx)),
        Poll::Ready(Some(Ok(_)))
    ));
    let polls_at_cancel = state.stream_polls.get();

    drop(stream);
    assert_eq!(state.dropped_streams.get(), 1);

    // Driving unrelated work afterwards must not resume the abandoned traversal.
    block_on(store.has(iri("alice"), knows, term("carol")).unwrap()).unwrap();

    assert_eq!(state.stream_polls.get(), polls_at_cancel);
    assert_eq!(state.emitted.get(), 1, "the remaining terms were abandoned");
    assert!(!state.drained.get());
}

#[test]
fn a_dropped_traversal_cancels_the_backend_request() {
    let (state, store) = delayed_store(1);
    let alice = store.node(iri("alice"));
    let knows = iri("knows");

    let mut stream = alice.out(&knows);
    assert_eq!(
        state.traversal(0),
        Traversal::Idle,
        "constructing a traversal must not start the backend request"
    );

    assert!(poll_once(|cx| stream.poll_next(cx)).is_pending());
    assert_eq!(
        state.traversal(0),
        Traversal::Active,
        "the first poll is what starts the request"
    );

    drop(stream);
    // The witness the poll counter cannot give: the cancellation reached the
    // backend's own token, not merely the wrapper.
    assert_eq!(
        state.traversal(0),
        Traversal::Cancelled,
        "dropping the wrapper stream must abandon the backend request"
    );
}

#[test]
fn an_exhausted_traversal_finishes_instead_of_cancelling() {
    let (state, store) = delayed_store(0);
    let alice = store.node(iri("alice"));
    let knows = iri("knows");

    let mut stream = alice.out(&knows);
    let count = block_on(async {
        let mut count = 0;
        while let Some(node) = stream.next().await {
            node.unwrap();
            count += 1;
        }
        count
    });

    assert_eq!(count, 3);
    assert_eq!(state.traversal(0), Traversal::Finished);

    drop(stream);
    assert_eq!(
        state.traversal(0),
        Traversal::Finished,
        "a result set consumed to its end is not a cancelled request"
    );
}

#[test]
fn async_add_has_delete_round_trip() {
    let state = Rc::new(FakeState::default());
    let store = AsyncStore::new(FakeStore {
        state: Rc::clone(&state),
        delay: 2,
    });
    let alice = iri("alice");
    let knows = iri("knows");
    let bob = term("bob");

    assert!(!block_on(
        store
            .has(alice.clone(), knows.clone(), bob.clone())
            .unwrap()
    )
    .unwrap());

    assert!(block_on(
        store
            .add(alice.clone(), knows.clone(), bob.clone())
            .unwrap(),
    )
    .unwrap());
    assert!(!block_on(
        store
            .add(alice.clone(), knows.clone(), bob.clone())
            .unwrap(),
    )
    .unwrap());
    assert!(block_on(
        store
            .has(alice.clone(), knows.clone(), bob.clone())
            .unwrap()
    )
    .unwrap());
    assert_eq!(state.triples.borrow().len(), 1);

    assert!(block_on(
        store
            .delete(alice.clone(), knows.clone(), bob.clone())
            .unwrap(),
    )
    .unwrap());
    assert!(!block_on(
        store
            .delete(alice.clone(), knows.clone(), bob.clone())
            .unwrap(),
    )
    .unwrap());
    assert!(!block_on(store.has(alice, knows, bob).unwrap()).unwrap());
    assert!(state.triples.borrow().is_empty());
}

#[test]
fn node_level_add_has_delete_round_trip() {
    let state = Rc::new(FakeState::default());
    let store = AsyncStore::new(FakeStore {
        state: Rc::clone(&state),
        delay: 1,
    });
    let alice = store.node(iri("alice"));
    let name = iri("name");
    let value = Literal::new_simple_literal("Alice");

    block_on(alice.add(name.clone(), value.clone()).unwrap()).unwrap();
    assert!(block_on(alice.has(name.clone(), value.clone()).unwrap()).unwrap());

    // The write is visible to a fresh traversal of the same store.
    let mut names = alice.out(&name);
    let found = block_on(names.next()).expect("one name").unwrap();
    assert_eq!(found.into_term(), Term::Literal(value.clone()));

    block_on(alice.delete(name.clone(), value.clone()).unwrap()).unwrap();
    assert!(!block_on(alice.has(name, value).unwrap()).unwrap());
}

// ---------------------------------------------------------------------------
// Surrounding wrapper behaviour.
// ---------------------------------------------------------------------------

#[test]
fn next_yields_every_node_in_producer_order_then_ends() {
    let (state, store) = delayed_store(1);
    let alice = store.node(iri("alice"));
    let knows = iri("knows");

    let focuses = block_on(async {
        let mut stream = alice.out(&knows);
        let mut focuses = Vec::new();
        while let Some(node) = stream.next().await {
            focuses.push(node.unwrap().into_term());
        }
        focuses
    });

    assert_eq!(focuses, vec![term("bob"), term("carol"), term("dave")]);
    assert_eq!(state.emitted.get(), 3);
    assert!(state.drained.get());
}

#[test]
fn in_traversal_streams_subjects_and_into_values_unwraps() {
    let (_, store) = delayed_store(0);
    let bob = store.node(term("bob"));
    let knows = iri("knows");

    let subjects = block_on(async {
        let mut stream = bob.r#in(&knows);
        let mut subjects = Vec::new();
        while let Some(node) = stream.next().await {
            subjects.push(node.unwrap().into_term());
        }
        subjects
    });
    assert_eq!(subjects, vec![term("alice")]);

    // `into_values` drops the wrappers and returns the raw backend stream.
    let mut values = Box::pin(bob.r#in(&knows).into_values());
    let first = poll_once(|cx| values.as_mut().poll_next(cx));
    assert_eq!(
        first.map(|item| item.unwrap().unwrap()),
        Poll::Ready(term("alice"))
    );
}

#[test]
fn traversal_of_an_absent_focus_is_empty() {
    let (state, store) = delayed_store(0);
    let mut stream = store.node(term("nobody")).out(&iri("knows"));

    assert!(matches!(
        poll_once(|cx| stream.poll_next(cx)),
        Poll::Ready(None)
    ));
    assert_eq!(state.emitted.get(), 0);
    assert!(state.drained.get());
}

#[test]
fn backend_read_errors_surface_through_the_wrapped_stream() {
    let (state, store) = delayed_store(0);
    state.fail_reads.set(true);
    let mut stream = store.node(iri("alice")).out(&iri("knows"));

    match poll_once(|cx| stream.poll_next(cx)) {
        Poll::Ready(Some(Err(error))) => {
            assert_eq!(error, AsyncStoreError::Backend("link down".to_owned()));
            assert_eq!(error.to_string(), "async store operation failed: link down");
        }
        _ => panic!("expected the backend error to reach the caller unwrapped"),
    }
}

#[test]
fn literal_subjects_are_rejected_before_the_backend_is_called() {
    let (state, store) = delayed_store(0);
    let literal = Literal::new_simple_literal("not a subject");
    let knows = iri("knows");

    for error in [
        store
            .add(literal.clone(), knows.clone(), term("bob"))
            .err()
            .map(|e| e.to_string()),
        store
            .has(literal.clone(), knows.clone(), term("bob"))
            .err()
            .map(|e| e.to_string()),
        store
            .delete(literal.clone(), knows.clone(), term("bob"))
            .err()
            .map(|e| e.to_string()),
        store
            .node(literal)
            .add(knows, term("bob"))
            .err()
            .map(|e| e.to_string()),
    ] {
        assert_eq!(
            error.as_deref(),
            Some("RDF literals cannot be triple subjects")
        );
    }

    assert_eq!(state.triples.borrow().len(), 3, "no write was attempted");
}

#[test]
fn wrapper_accessors_expose_the_backend_and_the_focus() {
    let (state, store) = delayed_store(0);
    assert_eq!(store.backend().state.triples.borrow().len(), 3);
    assert!(format!("{:?}", store).starts_with("AsyncStore"));

    let alice = store.node(iri("alice"));
    assert_eq!(alice.focus(), &term("alice"));
    assert_eq!(alice.clone().into_term(), term("alice"));
    assert!(format!("{:?}", alice).contains("alice"));
    assert!(format!("{:?}", alice.out(&iri("knows"))).starts_with("NodeStream"));
    // The node's store is the one it was created from.
    assert_eq!(alice.store().backend().state.triples.borrow().len(), 3);

    let backend = store.into_backend();
    assert!(Rc::ptr_eq(&backend.state, &state));
}

#[test]
fn a_boxed_pinned_backend_stream_is_a_term_stream() {
    let (state, store) = delayed_store(1);
    let stream = store.node(iri("alice")).out(&iri("knows")).into_values();
    // `Pin<Box<S>>` re-implements the trait, which is how a `!Unpin` backend
    // stream is made usable by the wrapper.
    let mut boxed: Pin<Box<dyn TermStream>> = Box::pin(stream);

    assert!(poll_once(|cx| boxed.as_mut().poll_next(cx)).is_pending());
    assert!(matches!(
        poll_once(|cx| boxed.as_mut().poll_next(cx)),
        Poll::Ready(Some(Ok(_)))
    ));
    assert_eq!(state.emitted.get(), 1);
}
