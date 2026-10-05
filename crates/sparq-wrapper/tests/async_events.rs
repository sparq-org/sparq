// sq-1rg2q.10: awaited effective-change listeners. Gated listeners prove the
// mutation future stays pending until every listener, in subscription order, releases.

#![cfg(feature = "proposed-async-events")]

mod common;

use common::{block_on, iri, term, FakeState, FakeStore, TestWaker};
use oxrdf::Literal;
use sparq_wrapper::proposed::async_events::AsyncObservableStore;
use sparq_wrapper::proposed::async_store::AsyncStoreError;
use sparq_wrapper::proposed::observe::{ChangeEvent, ChangeKind};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

#[derive(Default)]
struct Gate {
    open: Cell<bool>,
    waker: RefCell<Option<Waker>>,
}

impl Gate {
    fn release(&self) {
        self.open.set(true);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    async fn wait(&self) {
        std::future::poll_fn(|cx| {
            if self.open.get() {
                return Poll::Ready(());
            }
            *self.waker.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }
}

type Log = Rc<RefCell<Vec<String>>>;

fn gated_store() -> (
    Rc<FakeState>,
    AsyncObservableStore<FakeStore>,
    Log,
    [Rc<Gate>; 2],
) {
    let state = Rc::new(FakeState::default());
    let mut store = AsyncObservableStore::new(FakeStore {
        state: Rc::clone(&state),
        delay: 1,
    });
    let log = Log::default();
    let gates = [Rc::new(Gate::default()), Rc::new(Gate::default())];
    for (name, gate) in ["a", "b"].into_iter().zip(&gates) {
        let (log, gate) = (Rc::clone(&log), Rc::clone(gate));
        store.subscribe(move |event: ChangeEvent| {
            let (log, gate) = (Rc::clone(&log), Rc::clone(&gate));
            async move {
                log.borrow_mut()
                    .push(format!("enter {name} {:?}", event.kind));
                gate.wait().await;
                log.borrow_mut().push(format!("exit {name}"));
            }
        });
    }
    (state, store, log, gates)
}

/// Polls until the future settles or stalls without a wake-up.
fn drive<F: Future>(future: std::pin::Pin<&mut F>, signal: &Arc<TestWaker>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::clone(signal));
    let mut cx = Context::from_waker(&waker);
    let mut future = future;
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return Poll::Ready(output),
            Poll::Pending if signal.take_woken() => continue,
            Poll::Pending => return Poll::Pending,
        }
    }
}

#[test]
fn mutation_awaits_listeners_sequentially_in_subscription_order() {
    let (state, mut store, log, [a, b]) = gated_store();
    let signal = Arc::new(TestWaker::default());
    {
        let mut add = pin!(store.add(iri("alice"), iri("knows"), term("bob")));

        assert!(drive(add.as_mut(), &signal).is_pending());
        // The write is committed before notification; only listener a has entered.
        assert!(state.contains(&term("alice"), &iri("knows"), &term("bob")));
        assert_eq!(*log.borrow(), ["enter a Add"]);

        a.release();
        assert!(drive(add.as_mut(), &signal).is_pending());
        assert_eq!(*log.borrow(), ["enter a Add", "exit a", "enter b Add"]);

        b.release();
        assert_eq!(drive(add.as_mut(), &signal), Poll::Ready(Ok(true)));
    }
    assert_eq!(
        *log.borrow(),
        ["enter a Add", "exit a", "enter b Add", "exit b"]
    );

    // NON-VACUOUS: either no-op notifying would append records here.
    assert_eq!(
        block_on(store.add(iri("alice"), iri("knows"), term("bob"))),
        Ok(false)
    );
    assert_eq!(
        block_on(store.delete(iri("alice"), iri("knows"), term("carol"))),
        Ok(false)
    );
    assert_eq!(log.borrow().len(), 4);

    assert_eq!(
        block_on(store.delete(iri("alice"), iri("knows"), term("bob"))),
        Ok(true)
    );
    assert_eq!(
        log.borrow()[4..],
        ["enter a Delete", "exit a", "enter b Delete", "exit b"]
    );
    assert!(state.triples.borrow().is_empty());
}

#[test]
fn listeners_receive_the_event_and_can_unsubscribe() {
    let state = Rc::new(FakeState::default());
    let mut store = AsyncObservableStore::new(FakeStore {
        state: Rc::clone(&state),
        delay: 0,
    });
    let seen = Rc::new(RefCell::new(Vec::new()));
    let seen_by_listener = Rc::clone(&seen);
    let id = store.subscribe(move |event| {
        seen_by_listener.borrow_mut().push(event);
        std::future::ready(())
    });

    let name = Literal::new_simple_literal("Alice");
    assert_eq!(
        block_on(store.add(iri("alice"), iri("name"), name.clone())),
        Ok(true)
    );
    assert_eq!(
        *seen.borrow(),
        [ChangeEvent {
            kind: ChangeKind::Add,
            subject: term("alice"),
            predicate: iri("name"),
            object: name.into(),
        }]
    );

    assert!(store.unsubscribe(id));
    assert!(!store.unsubscribe(id));
    assert_eq!(
        block_on(store.delete(
            iri("alice"),
            iri("name"),
            Literal::new_simple_literal("Alice")
        )),
        Ok(true)
    );
    assert_eq!(seen.borrow().len(), 1);
    assert!(store.store().backend().state.triples.borrow().is_empty());
}

#[test]
fn rejected_mutations_notify_nobody() {
    let (state, mut store, log, _) = gated_store();
    let literal = Literal::new_simple_literal("not a subject");
    assert_eq!(
        block_on(store.add(literal, iri("p"), term("o"))),
        Err(AsyncStoreError::LiteralSubject)
    );
    assert!(state.triples.borrow().is_empty());
    assert!(log.borrow().is_empty());
    assert!(Rc::ptr_eq(&store.into_store().into_backend().state, &state));
}

/// Two wrappers over one shared backend. `first` checks presence and suspends
/// on its write; `second` makes the same change in between, so `first`'s
/// write changes nothing and must neither report a change nor notify.
fn shared_backend_race(kind: ChangeKind) -> (Poll<Result<bool, AsyncStoreError>>, usize) {
    let state = Rc::new(FakeState::default());
    if kind == ChangeKind::Delete {
        state
            .triples
            .borrow_mut()
            .push((term("alice"), iri("knows"), term("bob")));
    }
    let backend = || FakeStore {
        state: Rc::clone(&state),
        delay: 2,
    };
    let (mut first, mut second) = (
        AsyncObservableStore::new(backend()),
        AsyncObservableStore::new(backend()),
    );
    let notified = Rc::new(Cell::new(0));
    let counter = Rc::clone(&notified);
    first.subscribe(move |_| {
        counter.set(counter.get() + 1);
        std::future::ready(())
    });

    let signal = Arc::new(TestWaker::default());
    let waker = Waker::from(Arc::clone(&signal));
    let mut cx = Context::from_waker(&waker);
    type Mutation<'a> = std::pin::Pin<Box<dyn Future<Output = Result<bool, AsyncStoreError>> + 'a>>;
    let mut pending: Mutation<'_> = match kind {
        ChangeKind::Add => Box::pin(first.add(iri("alice"), iri("knows"), term("bob"))),
        ChangeKind::Delete => Box::pin(first.delete(iri("alice"), iri("knows"), term("bob"))),
    };
    // Suspend `first` inside its write, past any presence check it makes.
    while state.writes_issued.get() == 0 {
        assert!(pending.as_mut().poll(&mut cx).is_pending());
    }
    let raced = match kind {
        ChangeKind::Add => block_on(second.add(iri("alice"), iri("knows"), term("bob"))),
        ChangeKind::Delete => block_on(second.delete(iri("alice"), iri("knows"), term("bob"))),
    };
    assert_eq!(raced, Ok(true));
    let outcome = drive(std::pin::Pin::new(&mut pending), &signal);
    drop(pending);
    (outcome, notified.get())
}

#[test]
fn a_write_another_wrapper_already_made_is_not_reported_or_notified() {
    for kind in [ChangeKind::Add, ChangeKind::Delete] {
        assert_eq!(
            shared_backend_race(kind),
            (Poll::Ready(Ok(false)), 0),
            "{kind:?} raced on a shared backend"
        );
    }
}
