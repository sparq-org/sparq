// sq-1rg2q.9/.10: the delayed, instrumented async backend shared by the
// async_store, async_node, and async_events integration tests (moved verbatim from
// tests/async_store.rs). Each test crate uses a different subset of it.
#![allow(dead_code)]

use oxrdf::{NamedNode, Term};
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
pub struct TestWaker {
    woken: AtomicBool,
}

impl TestWaker {
    pub fn take_woken(&self) -> bool {
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

pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let signal = Arc::new(TestWaker::default());
    let waker = Waker::from(Arc::clone(&signal));
    let mut cx = Context::from_waker(&waker);
    for _ in 0..1_000 {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        assert!(
            signal.take_woken(),
            "future returned Pending without waking"
        );
    }
    panic!("future did not settle within the driver's poll budget");
}

/// Polls once with a throwaway waker, so a test can observe an intermediate
/// `Pending` instead of driving to completion.
pub fn poll_once<T>(step: impl FnOnce(&mut Context<'_>) -> Poll<T>) -> Poll<T> {
    let waker = Waker::from(Arc::new(TestWaker::default()));
    step(&mut Context::from_waker(&waker))
}

// ---------------------------------------------------------------------------
// A delayed, instrumented fake store standing in for a remote/disk backend.
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct FakeState {
    pub triples: RefCell<Vec<(Term, NamedNode, Term)>>,
    /// Total `poll_next` calls the wrapper forwarded to a term stream.
    pub stream_polls: Cell<usize>,
    /// Terms actually handed to the wrapper.
    pub emitted: Cell<usize>,
    /// Set when a term stream was polled all the way to its end.
    pub drained: Cell<bool>,
    pub dropped_streams: Cell<usize>,
    /// When set, every term stream fails on its first ready poll.
    pub fail_reads: Cell<bool>,
    /// One lifecycle token per traversal handed out, in construction order.
    pub traversals: RefCell<Vec<Rc<Cell<Traversal>>>>,
}

/// The backend-side lifecycle of one traversal — the token a contract-honouring
/// backend would use to track, and abandon, its remote request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Traversal {
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
    pub fn traversal(&self, index: usize) -> Traversal {
        self.traversals.borrow()[index].get()
    }

    pub fn contains(&self, subject: &Term, predicate: &NamedNode, object: &Term) -> bool {
        self.triples
            .borrow()
            .iter()
            .any(|(s, p, o)| s == subject && p == predicate && o == object)
    }
}

pub enum Pattern {
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
pub struct FakeStream {
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
        this.state
            .stream_polls
            .set(this.state.stream_polls.get() + 1);
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

pub type OpFn<T> = dyn FnOnce(&FakeState) -> Result<T, AsyncStoreError>;

/// A delayed write/ask operation; the effect lands only on the final poll.
pub struct FakeOp<T> {
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

pub struct FakeStore {
    pub state: Rc<FakeState>,
    pub delay: usize,
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
                state
                    .triples
                    .borrow_mut()
                    .push((subject, predicate, object));
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

pub fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{}", local)).unwrap()
}

pub fn term(local: &str) -> Term {
    Term::NamedNode(iri(local))
}

/// A store whose `alice knows {bob, carol, dave}` triples each cost one extra poll.
pub fn delayed_store(delay: usize) -> (Rc<FakeState>, AsyncStore<FakeStore>) {
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
