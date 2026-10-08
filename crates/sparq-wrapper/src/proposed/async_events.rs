//! Awaited effective-change events on the asynchronous wrapper surface.
//!
//! [`AsyncObservableStore`] is the asynchronous sibling of
//! [`ObservableStore`](super::observe::ObservableStore), following the
//! still-unlanded rdfjs/wrapper async events proposal in draft PR #99. A
//! mutation future performs the backend write, which reports whether it
//! changed the store, and only for a change awaits each listener's future in
//! subscription order; it resolves only after the last listener has finished.
//! A duplicate add or an absent delete notifies nobody.
//!
//! The change report comes from the backend write itself (see
//! [`AsyncStoreBackend`]), not from a separate presence check, so the
//! effective-change guarantee also holds when several wrappers or other
//! clients share one backend: of two racing identical writes, only the one
//! that changed the store reports `true` and notifies.
//!
//! Listener futures are boxed without a `Send` bound, matching the crate's
//! executor-free, single-threaded async surface.

// sq-1rg2q.10: sequentially awaited listeners over the observe.rs machinery.

use super::async_store::{AsyncStore, AsyncStoreBackend, AsyncStoreError};
use super::observe::{ChangeEvent, ChangeKind, Subscribers, SubscriptionId};
use oxrdf::{NamedNode, Term};
use std::future::Future;
use std::pin::Pin;

type Listener = dyn FnMut(ChangeEvent) -> Pin<Box<dyn Future<Output = ()>>>;

/// An [`AsyncStore`] whose effective mutations await subscribed listeners.
///
/// Only mutations made through [`add`](Self::add) and [`delete`](Self::delete)
/// notify; writes made directly through [`store`](Self::store) or the backend
/// bypass the listeners.
pub struct AsyncObservableStore<B> {
    store: AsyncStore<B>,
    listeners: Subscribers<Listener>,
}

impl<B> AsyncObservableStore<B> {
    /// Wraps an asynchronous backend.
    pub fn new(backend: B) -> Self {
        Self::from_store(AsyncStore::new(backend))
    }

    /// Wraps an existing asynchronous store.
    pub fn from_store(store: AsyncStore<B>) -> Self {
        Self {
            store,
            listeners: Subscribers::new(),
        }
    }

    /// Returns the wrapped store for traversal and unobserved access.
    pub fn store(&self) -> &AsyncStore<B> {
        &self.store
    }

    /// Consumes this wrapper and returns its store.
    pub fn into_store(self) -> AsyncStore<B> {
        self.store
    }

    /// Subscribes an asynchronous listener to every effective mutation.
    ///
    /// The listener receives an owned event and returns the future the
    /// mutation awaits before resolving or notifying the next listener.
    pub fn subscribe<F>(
        &mut self,
        mut listener: impl FnMut(ChangeEvent) -> F + 'static,
    ) -> SubscriptionId
    where
        F: Future<Output = ()> + 'static,
    {
        self.listeners
            .add(Box::new(move |event| Box::pin(listener(event))))
    }

    /// Removes a subscription and reports whether it was present.
    pub fn unsubscribe(&mut self, id: SubscriptionId) -> bool {
        self.listeners.remove(id)
    }
}

impl<B: AsyncStoreBackend> AsyncObservableStore<B> {
    /// Adds one triple and reports whether the store changed.
    ///
    /// Resolves after the write and every listener have completed.
    pub async fn add(
        &mut self,
        subject: impl Into<Term>,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<bool, AsyncStoreError> {
        self.change(ChangeKind::Add, subject.into(), predicate, object.into())
            .await
    }

    /// Deletes one triple and reports whether the store changed.
    ///
    /// Resolves after the write and every listener have completed.
    pub async fn delete(
        &mut self,
        subject: impl Into<Term>,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<bool, AsyncStoreError> {
        self.change(ChangeKind::Delete, subject.into(), predicate, object.into())
            .await
    }

    async fn change(
        &mut self,
        kind: ChangeKind,
        subject: Term,
        predicate: NamedNode,
        object: Term,
    ) -> Result<bool, AsyncStoreError> {
        let (s, p, o) = (subject.clone(), predicate.clone(), object.clone());
        let changed = match kind {
            ChangeKind::Add => self.store.add(s, p, o)?.await?,
            ChangeKind::Delete => self.store.delete(s, p, o)?.await?,
        };
        if !changed {
            return Ok(false);
        }

        let event = ChangeEvent {
            kind,
            subject,
            predicate,
            object,
        };
        for listener in self.listeners.iter_mut() {
            listener(event.clone()).await;
        }
        Ok(true)
    }
}
