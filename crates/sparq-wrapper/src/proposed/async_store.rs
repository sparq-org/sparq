//! Streaming asynchronous store wrapper for remote and disk-backed RDF.
//!
//! [`AsyncStore`] is the asynchronous sibling of the crate-root [`Store`]: it
//! wraps a backend whose reads are not available synchronously — an HTTP SPARQL
//! endpoint, a Solid pod, an out-of-core on-disk index — and exposes the same
//! focus/traverse shape over it. This is the Rust wrapper slice of the still
//! unlanded rdfjs/wrapper asynchronous store proposal in issue #10 and draft
//! PR #97.
//!
//! # Streaming and cancellation
//!
//! Traversal is poll-based end to end. [`AsyncNode::out`] and its `r#in`
//! counterpart return a [`NodeStream`] that wraps each backend term into an
//! [`AsyncNode`]
//! *as it arrives*; the wrapper never collects a result set before handing the
//! first node back.
//!
//! The wrapper's own half of the cancellation story is unconditional:
//! constructing a [`NodeStream`] does nothing but call the backend's
//! constructor, no term is buffered between polls, and dropping the stream
//! drops the backend stream and never polls or drains it again.
//!
//! Whether that also stops in-flight *remote* work is the backend's half.
//! [`AsyncStoreBackend`] requires an implementation to start no I/O before the
//! stream or future it returned is first polled, and to abandon that work when
//! it is dropped; for a backend meeting that obligation a dropped traversal is
//! a cancelled one, and a partially consumed remote result set is abandoned
//! rather than drained. A backend that ignores the contract — one that spawns
//! the request onto a runtime inside [`objects`](AsyncStoreBackend::objects),
//! say — keeps that work running no matter what the wrapper does, because the
//! wrapper holds no handle to it.
//!
//! The wrapper has no runtime dependency and never blocks: it contains no
//! executor, spawns nothing, and only ever forwards the caller's
//! [`Context`]. Any executor that can poll a [`Future`] can drive it.
//!
//! # Backends
//!
//! [`TermStream`] mirrors the shape of `futures_core::Stream` narrowed to
//! RDF terms, so a backend built on the async ecosystem can forward to its own
//! stream in one line. A `!Unpin` backend stream — an `async` generator, say —
//! should be exposed as `Pin<Box<S>>`, which is [`Unpin`] and implements
//! [`TermStream`] through the provided impl.
//!
//! [`Store`]: crate::Store

// [SONNET-4.6] sq-1rg2q.8: poll-based streaming wrapper; no executor, no eager collection.

use oxrdf::{NamedNode, Term};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// An asynchronous stream of RDF terms produced by a store backend.
///
/// This is the same contract as `futures_core::Stream` with
/// `Item = Result<Term, AsyncStoreError>`: [`poll_next`](Self::poll_next)
/// returns [`Poll::Pending`] after arranging a wake-up, `Ready(Some(..))` for
/// one available term, and `Ready(None)` once the result set is exhausted.
/// Implementations must not block the calling thread, and must not be polled
/// again after returning `Ready(None)`.
pub trait TermStream {
    /// Attempts to pull the next term out of the stream.
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Term, AsyncStoreError>>>;
}

impl<S: TermStream + ?Sized> TermStream for Pin<Box<S>> {
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Term, AsyncStoreError>>> {
        S::poll_next(self.get_mut().as_mut(), cx)
    }
}

/// A remote or disk-backed store that answers wrapper reads asynchronously.
///
/// Every method is lazy, and an implementation is required to keep it that
/// way: the method must return without performing I/O, the stream or future it
/// returns must start no backend work until it is first polled, and dropping
/// that stream or future must cancel or abandon whatever work it had started.
///
/// The compiler cannot check any of this, so it is an obligation on the
/// implementor rather than a property of the type — and the laziness and
/// drop-cancellation the wrapper documents hold exactly for backends that meet
/// it. An eagerly started operation is still expressible, but it breaks those
/// guarantees for every caller of the wrapper: the handle it returns no longer
/// has anything to cancel on drop.
pub trait AsyncStoreBackend {
    /// The stream of matched terms returned by the traversal methods.
    type Stream: TermStream;
    /// The future returned by [`add`](Self::add).
    type Add: Future<Output = Result<(), AsyncStoreError>>;
    /// The future returned by [`has`](Self::has).
    type Has: Future<Output = Result<bool, AsyncStoreError>>;
    /// The future returned by [`delete`](Self::delete).
    type Delete: Future<Output = Result<(), AsyncStoreError>>;

    /// Streams the objects of every `(subject, predicate, ?)` triple.
    fn objects(&self, subject: Term, predicate: NamedNode) -> Self::Stream;

    /// Streams the subjects of every `(?, predicate, object)` triple.
    fn subjects(&self, predicate: NamedNode, object: Term) -> Self::Stream;

    /// Adds one triple to the store.
    fn add(&self, subject: Term, predicate: NamedNode, object: Term) -> Self::Add;

    /// Reports whether the store contains one triple.
    fn has(&self, subject: Term, predicate: NamedNode, object: Term) -> Self::Has;

    /// Deletes one triple from the store, succeeding when it was absent.
    fn delete(&self, subject: Term, predicate: NamedNode, object: Term) -> Self::Delete;
}

/// An asynchronous store exposed through the object-wrapper API.
pub struct AsyncStore<B> {
    backend: B,
}

impl<B> fmt::Debug for AsyncStore<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsyncStore").finish_non_exhaustive()
    }
}

impl<B> AsyncStore<B> {
    /// Wraps an asynchronous backend.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Returns the wrapped backend, the escape hatch for backend-native calls.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Consumes the wrapper and returns its backend.
    pub fn into_backend(self) -> B {
        self.backend
    }
}

impl<B: AsyncStoreBackend> AsyncStore<B> {
    /// Wraps a focus term. The term may be absent from the store; traversal
    /// then yields an empty stream.
    pub fn node(&self, focus: impl Into<Term>) -> AsyncNode<'_, B> {
        AsyncNode {
            store: self,
            focus: focus.into(),
        }
    }

    /// Adds one triple, returning the backend future that performs the write.
    ///
    /// The subject position is validated synchronously, so a literal subject is
    /// rejected before the backend is asked to do any work.
    pub fn add(
        &self,
        subject: impl Into<Term>,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<B::Add, AsyncStoreError> {
        let subject = checked_subject(subject.into())?;
        Ok(self.backend.add(subject, predicate, object.into()))
    }

    /// Asks whether the store contains one triple.
    ///
    /// A literal subject is rejected synchronously rather than reported as a
    /// missing triple, because it is not a well-formed RDF subject at all.
    pub fn has(
        &self,
        subject: impl Into<Term>,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<B::Has, AsyncStoreError> {
        let subject = checked_subject(subject.into())?;
        Ok(self.backend.has(subject, predicate, object.into()))
    }

    /// Deletes one triple, returning the backend future that performs the write.
    pub fn delete(
        &self,
        subject: impl Into<Term>,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<B::Delete, AsyncStoreError> {
        let subject = checked_subject(subject.into())?;
        Ok(self.backend.delete(subject, predicate, object.into()))
    }
}

/// A focus term bound to an [`AsyncStore`].
///
/// The node owns its focus term and borrows the store, exactly like the
/// synchronous [`Node`](crate::Node). Traversals return streams whose items
/// borrow the same store, so chained navigation needs no clone of the wrapper.
pub struct AsyncNode<'store, B> {
    store: &'store AsyncStore<B>,
    focus: Term,
}

impl<B> Clone for AsyncNode<'_, B> {
    fn clone(&self) -> Self {
        Self {
            store: self.store,
            focus: self.focus.clone(),
        }
    }
}

impl<B> fmt::Debug for AsyncNode<'_, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsyncNode")
            .field("focus", &self.focus)
            .finish_non_exhaustive()
    }
}

impl<'store, B: AsyncStoreBackend> AsyncNode<'store, B> {
    /// Returns the wrapped focus term.
    pub fn focus(&self) -> &Term {
        &self.focus
    }

    /// Unwraps this node into its owned focus term.
    pub fn into_term(self) -> Term {
        self.focus
    }

    /// Returns the store this node is bound to.
    pub fn store(&self) -> &'store AsyncStore<B> {
        self.store
    }

    /// Streams `(focus, predicate, object)` objects as wrapped nodes.
    ///
    /// The wrapper does nothing here but build the backend stream, which under
    /// the [`AsyncStoreBackend`] contract performs no work until it is polled.
    pub fn out(&self, predicate: &NamedNode) -> NodeStream<'store, B> {
        NodeStream {
            store: self.store,
            inner: self
                .store
                .backend
                .objects(self.focus.clone(), predicate.clone()),
        }
    }

    /// Streams `(subject, predicate, focus)` subjects as wrapped nodes.
    ///
    /// The raw identifier is Rust's required spelling for a method named `in`.
    pub fn r#in(&self, predicate: &NamedNode) -> NodeStream<'store, B> {
        NodeStream {
            store: self.store,
            inner: self
                .store
                .backend
                .subjects(predicate.clone(), self.focus.clone()),
        }
    }

    /// Adds one outgoing triple from this focus.
    pub fn add(
        &self,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<B::Add, AsyncStoreError> {
        self.store.add(self.focus.clone(), predicate, object)
    }

    /// Asks whether one outgoing triple from this focus exists.
    pub fn has(
        &self,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<B::Has, AsyncStoreError> {
        self.store.has(self.focus.clone(), predicate, object)
    }

    /// Deletes one outgoing triple from this focus.
    pub fn delete(
        &self,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<B::Delete, AsyncStoreError> {
        self.store.delete(self.focus.clone(), predicate, object)
    }
}

/// A stream of nodes bound to one [`AsyncStore`].
///
/// Each backend term is wrapped as it arrives, so the first node is observable
/// long before the backend finishes producing. There is deliberately no
/// `collect`: buffering a remote result set is the caller's decision, not the
/// wrapper's. Dropping the stream drops the backend stream and never polls it
/// again, which cancels the traversal for any backend honouring the
/// [`AsyncStoreBackend`] laziness and drop-cancellation contract.
pub struct NodeStream<'store, B: AsyncStoreBackend> {
    store: &'store AsyncStore<B>,
    inner: B::Stream,
}

impl<B: AsyncStoreBackend> fmt::Debug for NodeStream<'_, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeStream").finish_non_exhaustive()
    }
}

impl<B: AsyncStoreBackend> NodeStream<'_, B> {
    /// Removes the node wrappers and returns the backend's term stream.
    pub fn into_values(self) -> B::Stream {
        self.inner
    }
}

impl<'store, B> NodeStream<'store, B>
where
    B: AsyncStoreBackend,
    B::Stream: Unpin,
{
    /// Attempts to pull the next wrapped node out of the stream.
    ///
    /// Forwards the caller's waker to the backend stream unchanged and wraps at
    /// most one term per call.
    pub fn poll_next(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<AsyncNode<'store, B>, AsyncStoreError>>> {
        let store = self.store;
        Pin::new(&mut self.inner)
            .poll_next(cx)
            .map(|item| item.map(|term| term.map(|focus| AsyncNode { store, focus })))
    }

    /// Resolves to the next wrapped node, or `None` at the end of the stream.
    ///
    /// Cancellation-safe: dropping the returned future between polls loses no
    /// term, because the wrapper buffers nothing of its own.
    pub async fn next(&mut self) -> Option<Result<AsyncNode<'store, B>, AsyncStoreError>> {
        std::future::poll_fn(|cx| self.poll_next(cx)).await
    }
}

/// An error from an asynchronous store operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncStoreError {
    /// RDF literals cannot occupy the subject position.
    LiteralSubject,
    /// The backing store reported a failure.
    Backend(String),
}

impl fmt::Display for AsyncStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LiteralSubject => f.write_str("RDF literals cannot be triple subjects"),
            Self::Backend(message) => write!(f, "async store operation failed: {}", message),
        }
    }
}

impl std::error::Error for AsyncStoreError {}

fn checked_subject(subject: Term) -> Result<Term, AsyncStoreError> {
    if subject.is_literal() {
        return Err(AsyncStoreError::LiteralSubject);
    }
    Ok(subject)
}
