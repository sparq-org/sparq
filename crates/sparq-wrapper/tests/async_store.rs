// [SONNET-4.6] sq-1rg2q.8: streaming, cancellation, and round-trip witnesses for the
// asynchronous store wrapper. The fake backend is deliberately delayed and instrumented so
// the assertions observe WHEN the producer ran, not just what it produced.

#![cfg(feature = "proposed-async-store")]

use oxrdf::{Literal, NamedNode, Term};
use sparq_wrapper::proposed::async_store::{
    AsyncStore, AsyncStoreBackend, AsyncStoreError, TermStream,
};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

// ---------------------------------------------------------------------------
// A minimal single-threaded driver. The crate ships no executor, so the tests
// bring their own: poll, and require a wake-up before every re-poll.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestWaker {
    woken: AtomicBool,
}

impl TestWaker {
    fn take_woken(&self) -> bool {
        self.woken.swap(false, Ordering::SeqCst)
    }
}

impl Wake for TestWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::SeqCst);
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let signal = Arc::new(TestWaker::default());
    let waker = Waker::from(Arc::clone(&signal));
    let mut cx = Context::from_waker(&waker);
    for _ in 0..1_000 {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        assert!(signal.take_woken(), "future returned Pending without waking");
    }
    panic!("future did not settle within the driver's poll budget");
}

/// Polls once with a throwaway waker, so a test can observe an intermediate
/// `Pending` instead of driving to completion.
fn poll_once<T>(step: impl FnOnce(&mut Context<'_>) -> Poll<T>) -> Poll<T> {
    let waker = Waker::from(Arc::new(TestWaker::default()));
    step(&mut Context::from_waker(&waker))
}

// ---------------------------------------------------------------------------
// A delayed, instrumented fake store standing in for a remote/disk backend.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeState {
    triples: RefCell<Vec<(Term, NamedNode, Term)>>,
    /// Total `poll_next` calls the wrapper forwarded to a term stream.
    stream_polls: Cell<usize>,
    /// Terms actually handed to the wrapper.
    emitted: Cell<usize>,
    /// Set when a term stream was polled all the way to its end.
    drained: Cell<bool>,
    dropped_streams: Cell<usize>,
    /// When set, every term stream fails on its first ready poll.
    fail_reads: Cell<bool>,
    /// One lifecycle token per traversal handed out, in construction order.
    traversals: RefCell<Vec<Rc<Cell<Traversal>>>>,
}

/// The backend-side lifecycle of one traversal — the token a contract-honouring
/// backend would use to track, and abandon, its remote request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Traversal {
    /// The stream exists but has never been polled, so no request has started.
    Idle,
    /// The request is live.
    Active,
    /// The stream was dropped mid-result-set, so the request was abandoned.
    Cancelled,
    /// The result set was consumed to its end.
    Finished,
}

impl FakeState {
    /// The lifecycle of the `index`-th traversal this backend handed out.
    fn traversal(&self, index: usize) -> Traversal {
        self.traversals.borrow()[index].get()
    }

    fn contains(&self, subject: &Term, predicate: &NamedNode, object: &Term) -> bool {
        self.triples
            .borrow()
            .iter()
            .any(|(s, p, o)| s == subject && p == predicate && o == object)
    }
}

enum Pattern {
    Objects(Term, NamedNode),
    Subjects(NamedNode, Term),
}

impl Pattern {
    fn matched(&self, triple: &(Term, NamedNode, Term)) -> Option<Term> {
        let (subject, predicate, object) = triple;
        match self {
            Self::Objects(s, p) => (s == subject && p == predicate).then(|| object.clone()),
            Self::Subjects(p, o) => (p == predicate && o == object).then(|| subject.clone()),
        }
    }
}

/// Yields one matching term every `delay + 1` polls, never all at once.
struct FakeStream {
    state: Rc<FakeState>,
    pattern: Pattern,
    cursor: usize,
    ticks: usize,
    delay: usize,
    token: Rc<Cell<Traversal>>,
}

impl TermStream for FakeStream {
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Term, AsyncStoreError>>> {
        let this = self.get_mut();
        this.state.stream_polls.set(this.state.stream_polls.get() + 1);
        // The first poll is what starts the request, exactly as the
        // `AsyncStoreBackend` laziness contract requires.
        if this.token.get() == Traversal::Idle {
            this.token.set(Traversal::Active);
        }
        if this.ticks > 0 {
            this.ticks -= 1;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        if this.state.fail_reads.get() {
            return Poll::Ready(Some(Err(AsyncStoreError::Backend("link down".to_owned()))));
        }

        let triples = this.state.triples.borrow();
        while this.cursor < triples.len() {
            let matched = this.pattern.matched(&triples[this.cursor]);
            this.cursor += 1;
            if let Some(term) = matched {
                this.ticks = this.delay;
                this.state.emitted.set(this.state.emitted.get() + 1);
                return Poll::Ready(Some(Ok(term)));
            }
        }
        this.state.drained.set(true);
        this.token.set(Traversal::Finished);
        Poll::Ready(None)
    }
}

impl Drop for FakeStream {
    fn drop(&mut self) {
        self.state
            .dropped_streams
            .set(self.state.dropped_streams.get() + 1);
        // A backend honouring the contract abandons the request it started.
        if self.token.get() == Traversal::Active {
            self.token.set(Traversal::Cancelled);
        }
    }
}

type OpFn<T> = dyn FnOnce(&FakeState) -> Result<T, AsyncStoreError>;

/// A delayed write/ask operation; the effect lands only on the final poll.
struct FakeOp<T> {
    state: Rc<FakeState>,
    ticks: usize,
    run: Option<Box<OpFn<T>>>,
}

impl<T> Future for FakeOp<T> {
    type Output = Result<T, AsyncStoreError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.ticks > 0 {
            this.ticks -= 1;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        let run = this.run.take().expect("operation polled after completion");
        Poll::Ready(run(&this.state))
    }
}

struct FakeStore {
    state: Rc<FakeState>,
    delay: usize,
}

impl FakeStore {
    fn op<T>(
        &self,
        run: impl FnOnce(&FakeState) -> Result<T, AsyncStoreError> + 'static,
    ) -> FakeOp<T> {
        FakeOp {
            state: Rc::clone(&self.state),
            ticks: self.delay,
            run: Some(Box::new(run)),
        }
    }

    fn stream(&self, pattern: Pattern) -> FakeStream {
        let token = Rc::new(Cell::new(Traversal::Idle));
        self.state.traversals.borrow_mut().push(Rc::clone(&token));
        FakeStream {
            state: Rc::clone(&self.state),
            pattern,
            cursor: 0,
            ticks: self.delay,
            delay: self.delay,
            token,
        }
    }
}

impl AsyncStoreBackend for FakeStore {
    type Stream = FakeStream;
    type Add = FakeOp<()>;
    type Has = FakeOp<bool>;
    type Delete = FakeOp<()>;

    fn objects(&self, subject: Term, predicate: NamedNode) -> Self::Stream {
        self.stream(Pattern::Objects(subject, predicate))
    }

    fn subjects(&self, predicate: NamedNode, object: Term) -> Self::Stream {
        self.stream(Pattern::Subjects(predicate, object))
    }

    fn add(&self, subject: Term, predicate: NamedNode, object: Term) -> Self::Add {
        self.op(move |state| {
            if !state.contains(&subject, &predicate, &object) {
                state.triples.borrow_mut().push((subject, predicate, object));
            }
            Ok(())
        })
    }

    fn has(&self, subject: Term, predicate: NamedNode, object: Term) -> Self::Has {
        self.op(move |state| Ok(state.contains(&subject, &predicate, &object)))
    }

    fn delete(&self, subject: Term, predicate: NamedNode, object: Term) -> Self::Delete {
        self.op(move |state| {
            state
                .triples
                .borrow_mut()
                .retain(|(s, p, o)| !(s == &subject && p == &predicate && o == &object));
            Ok(())
        })
    }
}

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{}", local)).unwrap()
}

fn term(local: &str) -> Term {
    Term::NamedNode(iri(local))
}

/// A store whose `alice knows {bob, carol, dave}` triples each cost one extra poll.
fn delayed_store(delay: usize) -> (Rc<FakeState>, AsyncStore<FakeStore>) {
    let state = Rc::new(FakeState::default());
    *state.triples.borrow_mut() = vec![
        (term("alice"), iri("knows"), term("bob")),
        (term("alice"), iri("knows"), term("carol")),
        (term("alice"), iri("knows"), term("dave")),
    ];
    let store = AsyncStore::new(FakeStore {
        state: Rc::clone(&state),
        delay,
    });
    (state, store)
}

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

    assert!(!block_on(store.has(alice.clone(), knows.clone(), bob.clone()).unwrap()).unwrap());

    block_on(store.add(alice.clone(), knows.clone(), bob.clone()).unwrap()).unwrap();
    assert!(block_on(store.has(alice.clone(), knows.clone(), bob.clone()).unwrap()).unwrap());
    assert_eq!(state.triples.borrow().len(), 1);

    block_on(store.delete(alice.clone(), knows.clone(), bob.clone()).unwrap()).unwrap();
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
    assert_eq!(first.map(|item| item.unwrap().unwrap()), Poll::Ready(term("alice")));
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
            assert_eq!(
                error.to_string(),
                "async store operation failed: link down"
            );
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
