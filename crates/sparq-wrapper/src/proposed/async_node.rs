//! Mapped cardinality reads and live mapped sets over an [`AsyncStore`].
//!
//! This is the asynchronous sibling of [`cardinality`](super::cardinality),
//! following the still-unlanded rdfjs/wrapper async node proposal in draft
//! PR #98. Term identity stays synchronous — mappers receive an [`AsyncNode`]
//! whose focus is already known — while every store read and write is awaited.
//!
//! [`required`] and [`optional`] pull at most two values from the streamed
//! traversal: a second value already proves the cardinality violation, so the
//! rest of a remote result set is abandoned rather than drained. The reported
//! [`CardinalityError::found`] is then the lower bound `2`.

// [OPUS-5.5] sq-1rg2q.9: async mapped reads + live sets over the async_store surface.

use super::async_store::{AsyncNode, AsyncStore, AsyncStoreBackend, AsyncStoreError};
use crate::{Cardinality, CardinalityError};
use oxrdf::{NamedNode, Term};
use std::fmt;
use std::marker::PhantomData;

/// An error from an asynchronous mapped read or live-set operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsyncMapError<E> {
    /// The backing store reported a failure.
    Store(AsyncStoreError),
    /// The streamed values did not satisfy the requested cardinality.
    Cardinality(CardinalityError),
    /// A value could not be converted between RDF and the application type.
    Conversion(E),
}

impl<E: fmt::Display> fmt::Display for AsyncMapError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(f),
            Self::Cardinality(error) => error.fmt(f),
            Self::Conversion(error) => write!(f, "mapped value conversion failed: {error}"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for AsyncMapError<E> {}

impl<E> From<AsyncStoreError> for AsyncMapError<E> {
    fn from(error: AsyncStoreError) -> Self {
        Self::Store(error)
    }
}

/// Maps the exactly-one outgoing value for `predicate`.
///
/// The mapper runs only after the exactly-one check succeeds.
pub async fn required<'s, B, T, E>(
    node: &AsyncNode<'s, B>,
    predicate: &NamedNode,
    map: impl FnOnce(AsyncNode<'s, B>) -> Result<T, E>,
) -> Result<T, AsyncMapError<E>>
where
    B: AsyncStoreBackend,
    B::Stream: Unpin,
{
    singular(node, predicate, Cardinality::ExactlyOne)
        .await?
        .map(|value| map(value).map_err(AsyncMapError::Conversion))
        .expect("exactly-one cardinality returns a node")
}

/// Maps the zero-or-one outgoing value for `predicate`.
///
/// The mapper is not invoked for the empty case.
pub async fn optional<'s, B, T, E>(
    node: &AsyncNode<'s, B>,
    predicate: &NamedNode,
    map: impl FnOnce(AsyncNode<'s, B>) -> Result<T, E>,
) -> Result<Option<T>, AsyncMapError<E>>
where
    B: AsyncStoreBackend,
    B::Stream: Unpin,
{
    singular(node, predicate, Cardinality::AtMostOne)
        .await?
        .map(map)
        .transpose()
        .map_err(AsyncMapError::Conversion)
}

/// Maps every streamed outgoing value for `predicate`, in stream order.
///
/// Collecting is the caller's explicit choice here; use
/// [`AsyncNode::out`] directly to stay streaming.
pub async fn many<'s, B, T, E>(
    node: &AsyncNode<'s, B>,
    predicate: &NamedNode,
    mut map: impl FnMut(AsyncNode<'s, B>) -> Result<T, E>,
) -> Result<Vec<T>, AsyncMapError<E>>
where
    B: AsyncStoreBackend,
    B::Stream: Unpin,
{
    let mut stream = node.out(predicate);
    let mut values = Vec::new();
    while let Some(value) = stream.next().await {
        values.push(map(value?).map_err(AsyncMapError::Conversion)?);
    }
    Ok(values)
}

async fn singular<'s, B, E>(
    node: &AsyncNode<'s, B>,
    predicate: &NamedNode,
    expected: Cardinality,
) -> Result<Option<AsyncNode<'s, B>>, AsyncMapError<E>>
where
    B: AsyncStoreBackend,
    B::Stream: Unpin,
{
    let mut stream = node.out(predicate);
    let first = stream.next().await.transpose()?;
    let found = match first {
        None => 0,
        // Stop after the second value: it already decides the violation.
        Some(_) => 1 + usize::from(stream.next().await.transpose()?.is_some()),
    };
    if found == 2 || (found == 0 && expected == Cardinality::ExactlyOne) {
        return Err(AsyncMapError::Cardinality(CardinalityError {
            focus: node.focus().clone(),
            predicate: predicate.clone(),
            expected,
            found,
        }));
    }
    Ok(first)
}

/// Creates a live mapped set over `(focus, predicate, ?object)` in `store`.
///
/// Every read re-streams the store and every mutation is awaited against it,
/// so the view never caches a stale result set.
pub fn live_set<'s, B, T, Decode, Encode>(
    store: &'s AsyncStore<B>,
    focus: impl Into<Term>,
    predicate: NamedNode,
    decode: Decode,
    encode: Encode,
) -> AsyncLiveSet<'s, B, T, Decode, Encode> {
    AsyncLiveSet {
        store,
        focus: focus.into(),
        predicate,
        decode,
        encode,
        value: PhantomData,
    }
}

/// A live mapped set backed by outgoing triples in an [`AsyncStore`].
///
/// The decoder maps streamed nodes to application values; the encoder maps an
/// application value back to its RDF object term. Encoding completes before
/// any store call, so a failed conversion never reaches the backend.
/// `insert` and `remove` check presence, then write: the pair is not atomic
/// against other writers to the same backend.
pub struct AsyncLiveSet<'s, B, T, Decode, Encode> {
    store: &'s AsyncStore<B>,
    focus: Term,
    predicate: NamedNode,
    decode: Decode,
    encode: Encode,
    value: PhantomData<fn() -> T>,
}

impl<'s, B, T, Decode, Encode> AsyncLiveSet<'s, B, T, Decode, Encode>
where
    B: AsyncStoreBackend,
    B::Stream: Unpin,
{
    /// Streams and maps the set's current RDF terms.
    pub async fn values<E>(&self) -> Result<Vec<T>, AsyncMapError<E>>
    where
        Decode: Fn(AsyncNode<'s, B>) -> Result<T, E>,
    {
        many(
            &self.store.node(self.focus.clone()),
            &self.predicate,
            &self.decode,
        )
        .await
    }

    /// Reports whether the encoded value's RDF term is currently present.
    pub async fn contains<E>(&self, value: &T) -> Result<bool, AsyncMapError<E>>
    where
        Encode: Fn(&T) -> Result<Term, E>,
    {
        let has = self
            .node()
            .has(self.predicate.clone(), self.encode(value)?)?;
        Ok(has.await?)
    }

    /// Inserts the encoded RDF term and reports whether the set changed.
    pub async fn insert<E>(&self, value: &T) -> Result<bool, AsyncMapError<E>>
    where
        Encode: Fn(&T) -> Result<Term, E>,
    {
        let term = self.encode(value)?;
        let node = self.node();
        if node.has(self.predicate.clone(), term.clone())?.await? {
            return Ok(false);
        }
        node.add(self.predicate.clone(), term)?.await?;
        Ok(true)
    }

    /// Removes the encoded RDF term and reports whether the set changed.
    pub async fn remove<E>(&self, value: &T) -> Result<bool, AsyncMapError<E>>
    where
        Encode: Fn(&T) -> Result<Term, E>,
    {
        let term = self.encode(value)?;
        let node = self.node();
        if !node.has(self.predicate.clone(), term.clone())?.await? {
            return Ok(false);
        }
        node.delete(self.predicate.clone(), term)?.await?;
        Ok(true)
    }

    fn node(&self) -> AsyncNode<'s, B> {
        self.store.node(self.focus.clone())
    }

    fn encode<E>(&self, value: &T) -> Result<Term, AsyncMapError<E>>
    where
        Encode: Fn(&T) -> Result<Term, E>,
    {
        (self.encode)(value).map_err(AsyncMapError::Conversion)
    }
}
