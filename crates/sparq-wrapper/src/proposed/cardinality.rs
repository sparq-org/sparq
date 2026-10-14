//! Explicit mapped cardinalities and a live write-through collection.
//!
//! This is a Rust port of the unlanded rdfjs/wrapper arity proposal in issue
//! #8 and the live `WrappingSet` exposed by draft PR #92. Reads are evaluated
//! against the graph at call time. A [`LiveMappedCollection`] therefore stays
//! current after its own mutations without being reconstructed.

// sq-1rg2q.3: explicit RDF arities and mutation-safe live mappings.

use crate::{CardinalityError, Node, Store, StoreError};
use oxrdf::{NamedNode, Term};
use std::fmt;
use std::marker::PhantomData;

/// An error produced while evaluating a required or optional mapped value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardinalityViewError<E> {
    /// The RDF values did not satisfy the requested cardinality.
    Cardinality(CardinalityError),
    /// The selected RDF node could not be converted to the mapped value.
    Conversion(E),
}

impl<E: fmt::Display> fmt::Display for CardinalityViewError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cardinality(error) => error.fmt(f),
            Self::Conversion(error) => write!(f, "mapped value conversion failed: {error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for CardinalityViewError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cardinality(error) => Some(error),
            Self::Conversion(error) => Some(error),
        }
    }
}

/// An error produced while mapping a value for a live-collection operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionError<E> {
    /// The application value could not be converted to an RDF term.
    Conversion(E),
    /// The backing wrapper store rejected the mutation.
    Store(StoreError),
}

impl<E: fmt::Display> fmt::Display for CollectionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conversion(error) => write!(f, "RDF term conversion failed: {error}"),
            Self::Store(error) => error.fmt(f),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for CollectionError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Conversion(error) => Some(error),
            Self::Store(error) => Some(error),
        }
    }
}

/// Maps the exactly-one outgoing value for `predicate`.
///
/// Zero or multiple values produce the M1 [`CardinalityError`] wrapped in
/// [`CardinalityViewError::Cardinality`]. Conversion runs only after the
/// exactly-one check succeeds.
pub fn required<'g, T, E>(
    node: &Node<'g>,
    predicate: &NamedNode,
    map: impl FnOnce(Node<'g>) -> Result<T, E>,
) -> Result<T, CardinalityViewError<E>> {
    let value = node
        .required_out(predicate)
        .map_err(CardinalityViewError::Cardinality)?;
    map(value).map_err(CardinalityViewError::Conversion)
}

/// Maps the zero-or-one outgoing value for `predicate`.
///
/// Multiple values produce the M1 [`CardinalityError`]. The mapper is not
/// invoked for the empty case.
pub fn optional<'g, T, E>(
    node: &Node<'g>,
    predicate: &NamedNode,
    map: impl FnOnce(Node<'g>) -> Result<T, E>,
) -> Result<Option<T>, CardinalityViewError<E>> {
    node.optional_out(predicate)
        .map_err(CardinalityViewError::Cardinality)?
        .map(map)
        .transpose()
        .map_err(CardinalityViewError::Conversion)
}

/// Maps every distinct outgoing RDF term for `predicate`.
///
/// Results are collected in graph traversal order. The result is a `Vec`
/// rather than a mapped set so distinct RDF terms are never collapsed when a
/// non-injective mapper produces equal application values.
pub fn many<'g, T, E>(
    node: &Node<'g>,
    predicate: &NamedNode,
    map: impl FnMut(Node<'g>) -> Result<T, E>,
) -> Result<Vec<T>, E> {
    node.out(predicate).map(map).collect()
}

/// Creates a live mapped collection over `(focus, predicate, ?object)`.
///
/// The returned view borrows an owned [`Store`] mutably. Each read re-traverses
/// that store, while inserts, removals, and clears write through to it.
pub fn live_mapped<'store, 'graph, T, Decode, Encode>(
    store: &'store mut Store<'graph>,
    focus: impl Into<Term>,
    predicate: NamedNode,
    decode: Decode,
    encode: Encode,
) -> LiveMappedCollection<'store, 'graph, T, Decode, Encode> {
    LiveMappedCollection {
        store,
        focus: focus.into(),
        predicate,
        decode,
        encode,
        value: PhantomData,
    }
}

/// A live mapped collection backed by outgoing triples in an owned store.
///
/// The decoder maps RDF nodes to application values. The encoder maps an
/// application value back to its RDF object term. Conversion is completed
/// before a mutation begins, so a failed conversion leaves the store intact.
pub struct LiveMappedCollection<'store, 'graph, T, Decode, Encode> {
    store: &'store mut Store<'graph>,
    focus: Term,
    predicate: NamedNode,
    decode: Decode,
    encode: Encode,
    value: PhantomData<fn() -> T>,
}

impl<T, Decode, Encode> LiveMappedCollection<'_, '_, T, Decode, Encode> {
    /// Maps the collection's current RDF terms.
    pub fn values<E>(&self) -> Result<Vec<T>, E>
    where
        Decode: for<'node> Fn(Node<'node>) -> Result<T, E>,
    {
        self.store
            .node(self.focus.clone())
            .out(&self.predicate)
            .map(&self.decode)
            .collect()
    }

    /// Returns the number of distinct RDF object terms currently in the view.
    pub fn len(&self) -> usize {
        self.store
            .node(self.focus.clone())
            .out(&self.predicate)
            .len()
    }

    /// Returns whether the view currently has no RDF object terms.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns whether the encoded value's RDF term is currently present.
    pub fn contains<E>(&self, value: &T) -> Result<bool, CollectionError<E>>
    where
        Encode: Fn(&T) -> Result<Term, E>,
    {
        let term = (self.encode)(value).map_err(CollectionError::Conversion)?;
        Ok(self.contains_term(&term))
    }

    /// Inserts the encoded RDF term and reports whether the collection changed.
    pub fn insert<E>(&mut self, value: &T) -> Result<bool, CollectionError<E>>
    where
        Encode: Fn(&T) -> Result<Term, E>,
    {
        let term = (self.encode)(value).map_err(CollectionError::Conversion)?;
        let changed = !self.contains_term(&term);
        self.store
            .insert(self.focus.clone(), self.predicate.clone(), term)
            .map_err(CollectionError::Store)?;
        Ok(changed)
    }

    /// Removes the encoded RDF term and reports whether the collection changed.
    pub fn remove<E>(&mut self, value: &T) -> Result<bool, CollectionError<E>>
    where
        Encode: Fn(&T) -> Result<Term, E>,
    {
        let term = (self.encode)(value).map_err(CollectionError::Conversion)?;
        let changed = self.contains_term(&term);
        self.store
            .remove(self.focus.clone(), self.predicate.clone(), term)
            .map_err(CollectionError::Store)?;
        Ok(changed)
    }

    /// Removes all current RDF terms and returns the number removed.
    pub fn clear(&mut self) -> Result<usize, StoreError> {
        let terms: Vec<_> = self
            .store
            .node(self.focus.clone())
            .out(&self.predicate)
            .values()
            .collect();
        for term in &terms {
            self.store
                .remove(self.focus.clone(), self.predicate.clone(), term.clone())?;
        }
        Ok(terms.len())
    }

    fn contains_term(&self, expected: &Term) -> bool {
        self.store
            .node(self.focus.clone())
            .out(&self.predicate)
            .values()
            .any(|term| term == *expected)
    }
}
