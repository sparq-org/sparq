// sq-1rg2q.2: typed focus kinds + bound node factories (rdfjs/wrapper #83-#87).

//! **PROPOSED**: typed focus kinds and bound node factories.
//!
//! Rust port of the still-unlanded rdfjs/wrapper typed-wrapper series:
//!
//! - `TermWrapper.from` static factory with an intersection return type
//!   (<https://github.com/rdfjs/wrapper/pull/83>) → [`NodeFactory::term`],
//!   which preserves the concrete focus kind in [`AnyNode`].
//! - `TermWrapper.in` curried static factory that binds the dataset once
//!   (<https://github.com/rdfjs/wrapper/pull/84>) → [`NodeFactory`], a `Copy`
//!   handle bound to one borrowed graph that wraps many terms.
//! - `TermWrapper<T extends Term>` generic over the wrapped term
//!   (<https://github.com/rdfjs/wrapper/pull/85>) → [`TypedNode`], generic
//!   over a [`FocusKind`].
//! - Narrowing the member surface so no wrapper declares "false types"
//!   (<https://github.com/rdfjs/wrapper/pull/86>) → position-specific methods
//!   exist only on the kinds where they are meaningful.
//! - Position-typed dataset helper returns (`T & Quad_Subject`,
//!   `T & Quad_Object`, <https://github.com/rdfjs/wrapper/pull/87>) → the
//!   [`SubjectFocus`] and [`PredicateFocus`] capability traits.
//!
//! Where the TypeScript proposals use intersection types to make illegal term
//! positions a build error, this port uses sealed marker kinds and capability
//! traits: outgoing traversal from a literal focus does not compile, turning
//! the untyped wrapper's run-time `StoreError::LiteralSubject` contract into
//! a compile-time guarantee. Predicate-position helpers exist only on IRI
//! foci. Triple terms follow the RDF/JS `Quad_Subject` convention and this
//! crate's run-time rule (only literals are rejected as subjects): they are
//! legal subjects, never predicates.
//!
//! Factories bind borrowed store state only — a [`NodeFactory`] is one shared
//! graph reference, so constructing it never clones a term, copies a triple,
//! or materializes any dataset view.

use std::fmt;

use oxrdf::{BlankNode, Literal, NamedNode, Term, Triple};
use sparq_core::Graph;

use crate::{Dataset, Node, NodeSet, Store, TermKind};

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::IriFocus {}
    impl Sealed for super::BlankFocus {}
    impl Sealed for super::LiteralFocus {}
    impl Sealed for super::TripleFocus {}
}

/// A compile-time focus kind: the concrete RDF term type a [`TypedNode`] wraps.
///
/// This trait is sealed; its only implementors are the four marker kinds
/// [`IriFocus`], [`BlankFocus`], [`LiteralFocus`], and [`TripleFocus`].
pub trait FocusKind: sealed::Sealed {
    /// The concrete term type carried by this focus kind.
    type Term: Clone + fmt::Debug;

    /// The coarse term kind, for diagnostics.
    const KIND: TermKind;

    /// Converts the kind's concrete term into a generic [`Term`].
    fn into_term(focus: Self::Term) -> Term;
}

/// Focus kinds that are legal in the subject position of a triple.
///
/// Implemented for [`IriFocus`], [`BlankFocus`], and [`TripleFocus`] — never
/// for [`LiteralFocus`]: RDF literals cannot be triple subjects, so subject
/// traversal from a literal focus is a compile error instead of the untyped
/// wrapper's run-time `StoreError::LiteralSubject`.
pub trait SubjectFocus: FocusKind {}

/// Focus kinds that are legal in the predicate position of a triple.
///
/// Implemented only for [`IriFocus`]: RDF predicates are always IRIs.
pub trait PredicateFocus: FocusKind {}

/// Marker kind for an IRI focus. Legal in every triple position.
#[derive(Debug, Clone, Copy)]
pub enum IriFocus {}

impl FocusKind for IriFocus {
    type Term = NamedNode;
    const KIND: TermKind = TermKind::NamedNode;

    fn into_term(focus: NamedNode) -> Term {
        Term::NamedNode(focus)
    }
}

impl SubjectFocus for IriFocus {}
impl PredicateFocus for IriFocus {}

/// Marker kind for a blank-node focus. Legal as subject or object.
#[derive(Debug, Clone, Copy)]
pub enum BlankFocus {}

impl FocusKind for BlankFocus {
    type Term = BlankNode;
    const KIND: TermKind = TermKind::BlankNode;

    fn into_term(focus: BlankNode) -> Term {
        Term::BlankNode(focus)
    }
}

impl SubjectFocus for BlankFocus {}

/// Marker kind for a literal focus. Legal only in the object position.
#[derive(Debug, Clone, Copy)]
pub enum LiteralFocus {}

impl FocusKind for LiteralFocus {
    type Term = Literal;
    const KIND: TermKind = TermKind::Literal;

    fn into_term(focus: Literal) -> Term {
        Term::Literal(focus)
    }
}

/// Marker kind for an RDF 1.2 triple-term focus. Legal as subject or object.
#[derive(Debug, Clone, Copy)]
pub enum TripleFocus {}

impl FocusKind for TripleFocus {
    type Term = Triple;
    const KIND: TermKind = TermKind::Triple;

    fn into_term(focus: Triple) -> Term {
        Term::from(focus)
    }
}

impl SubjectFocus for TripleFocus {}

/// A focus term of a statically known kind, bound to a borrowed sparq graph.
///
/// The typed twin of [`Node`]: the wrapped term's kind is a type parameter,
/// so only position-legal operations exist. [`TypedNode::out`] requires
/// `K: SubjectFocus` and [`TypedNode::subjects`] requires
/// `K: PredicateFocus`, while the incoming traversal `TypedNode::in` exists
/// for every kind (every RDF term may appear in the object position).
pub struct TypedNode<'g, K: FocusKind> {
    graph: &'g Graph,
    focus: K::Term,
}

impl<K: FocusKind> Clone for TypedNode<'_, K> {
    fn clone(&self) -> Self {
        Self {
            graph: self.graph,
            focus: self.focus.clone(),
        }
    }
}

impl<K: FocusKind> fmt::Debug for TypedNode<'_, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypedNode")
            .field("kind", &K::KIND)
            .field("focus", &self.focus)
            .finish_non_exhaustive()
    }
}

impl<'g, K: FocusKind> TypedNode<'g, K> {
    /// Returns the wrapped focus term at its concrete type.
    pub fn focus(&self) -> &K::Term {
        &self.focus
    }

    /// Unwraps this node into its concrete focus term.
    pub fn into_focus(self) -> K::Term {
        self.focus
    }

    /// Returns the focus as a generic [`Term`].
    pub fn term(&self) -> Term {
        K::into_term(self.focus.clone())
    }

    /// Returns the coarse kind of the focus term.
    pub fn kind(&self) -> TermKind {
        K::KIND
    }

    /// Returns this node's dataset escape hatch.
    pub fn dataset(&self) -> Dataset<'g> {
        Dataset { graph: self.graph }
    }

    /// Erases the focus kind, yielding the untyped [`Node`] wrapper.
    pub fn into_node(self) -> Node<'g> {
        Node {
            graph: self.graph,
            focus: K::into_term(self.focus),
        }
    }

    /// Returns an untyped [`Node`] wrapper over a clone of the focus.
    pub fn as_node(&self) -> Node<'g> {
        self.clone().into_node()
    }

    /// Traverses `(subject, predicate, focus)` and returns the matched
    /// subjects. Available for every focus kind: any RDF term may appear in
    /// the object position.
    pub fn r#in(&self, predicate: &NamedNode) -> NodeSet<'g> {
        self.as_node().r#in(predicate)
    }
}

impl<'g, K: SubjectFocus> TypedNode<'g, K> {
    /// Traverses `(focus, predicate, object)` and returns the matched objects.
    ///
    /// Only focus kinds that are legal triple subjects have this method; a
    /// literal focus fails to compile (see [`NodeFactory::literal`]).
    pub fn out(&self, predicate: &NamedNode) -> NodeSet<'g> {
        self.as_node().out(predicate)
    }
}

impl<'g, K: PredicateFocus> TypedNode<'g, K> {
    /// Returns the subject of every triple whose predicate is this focus.
    ///
    /// Only predicate-legal (IRI) foci have this method. Duplicates are kept;
    /// the distinct variants are the separate `proposed-distinct` feature.
    pub fn subjects(&self) -> NodeSet<'g> {
        self.predicate_position(0)
    }

    /// Returns the object of every triple whose predicate is this focus.
    ///
    /// Only predicate-legal (IRI) foci have this method. Duplicates are kept;
    /// the distinct variants are the separate `proposed-distinct` feature.
    pub fn objects(&self) -> NodeSet<'g> {
        self.predicate_position(2)
    }

    fn predicate_position(&self, position: usize) -> NodeSet<'g> {
        let Some(predicate) = self.graph.id_of(&K::into_term(self.focus.clone())) else {
            return NodeSet::empty(self.graph);
        };
        let scan = self.graph.store.scan(&[None, Some(predicate), None]);
        let terms = scan
            .rows
            .iter()
            .map(|row| self.graph.dict.term(scan.to_spo(row)[position]))
            .collect();
        NodeSet::new(self.graph, terms)
    }
}

/// A node factory bound to one borrowed graph.
///
/// The Rust port of the curried `TermWrapper.in(dataset, factory)` proposal
/// (rdfjs/wrapper draft PR #84): bind the store once, then wrap many terms.
/// The factory is a single shared graph reference — `Copy`, and constructing
/// or using it never clones the graph or materializes a dataset view.
///
/// ```
/// use oxrdf::{Literal, NamedNode, Term};
/// use sparq_core::Graph;
/// use sparq_wrapper::proposed::typed_focus::NodeFactory;
///
/// let mut graph = Graph::new();
/// let alice = NamedNode::new("http://example.org/alice")?;
/// let name = NamedNode::new("http://example.org/name")?;
/// graph.insert_triple(alice.clone(), name.clone(), Literal::new_simple_literal("Alice"))?;
///
/// // One bound factory wraps many terms over the same borrowed graph.
/// let factory = NodeFactory::new(&graph);
/// let subject = factory.iri(alice);
/// let object = factory.literal(Literal::new_simple_literal("Alice"));
///
/// assert_eq!(subject.out(&name).len(), 1);
/// assert_eq!(object.r#in(&name).len(), 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// A literal focus is not a legal subject, so outgoing traversal from it does
/// not compile:
///
/// ```compile_fail
/// use oxrdf::{Literal, NamedNode};
/// use sparq_core::Graph;
/// use sparq_wrapper::proposed::typed_focus::NodeFactory;
///
/// let graph = Graph::new();
/// let name = NamedNode::new("http://example.org/name").unwrap();
/// let literal = NodeFactory::new(&graph).literal(Literal::new_simple_literal("Alice"));
/// let _ = literal.out(&name); // ERROR: `LiteralFocus` is not a `SubjectFocus`
/// ```
#[derive(Clone, Copy)]
pub struct NodeFactory<'g> {
    graph: &'g Graph,
}

impl fmt::Debug for NodeFactory<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeFactory").finish_non_exhaustive()
    }
}

impl<'g> NodeFactory<'g> {
    /// Binds a factory to a borrowed graph.
    pub fn new(graph: &'g Graph) -> Self {
        Self { graph }
    }

    /// Binds a factory to the graph wrapped by a [`Store`].
    pub fn for_store(store: &'g Store<'_>) -> Self {
        Self::new(store.graph())
    }

    /// Binds a factory to the graph behind a [`Dataset`] view.
    pub fn for_dataset(dataset: Dataset<'g>) -> Self {
        Self::new(dataset.graph())
    }

    /// Returns the bound graph.
    pub fn graph(self) -> &'g Graph {
        self.graph
    }

    /// Wraps an IRI focus.
    pub fn iri(self, focus: NamedNode) -> TypedNode<'g, IriFocus> {
        self.wrap(focus)
    }

    /// Wraps a blank-node focus.
    pub fn blank(self, focus: BlankNode) -> TypedNode<'g, BlankFocus> {
        self.wrap(focus)
    }

    /// Wraps a literal focus.
    ///
    /// The result has no `out` traversal: literals are not legal triple
    /// subjects, and [`TypedNode`] enforces that at compile time.
    pub fn literal(self, focus: Literal) -> TypedNode<'g, LiteralFocus> {
        self.wrap(focus)
    }

    /// Wraps an RDF 1.2 triple-term focus.
    pub fn triple(self, focus: Triple) -> TypedNode<'g, TripleFocus> {
        self.wrap(focus)
    }

    /// Wraps any term, preserving its concrete kind in the returned
    /// [`AnyNode`] — the port of the `TermWrapper.from` overload set
    /// (rdfjs/wrapper draft PR #83).
    pub fn term(self, focus: impl Into<Term>) -> AnyNode<'g> {
        match focus.into() {
            Term::NamedNode(named) => AnyNode::Iri(self.iri(named)),
            Term::BlankNode(blank) => AnyNode::Blank(self.blank(blank)),
            Term::Literal(literal) => AnyNode::Literal(self.literal(literal)),
            Term::Triple(triple) => AnyNode::Triple(self.triple(*triple)),
        }
    }

    fn wrap<K: FocusKind>(self, focus: K::Term) -> TypedNode<'g, K> {
        TypedNode {
            graph: self.graph,
            focus,
        }
    }
}

/// A typed node of a kind established at run time by [`NodeFactory::term`].
///
/// Matching on the variant recovers the compile-time focus kind, after which
/// only position-legal operations are available.
#[derive(Debug, Clone)]
pub enum AnyNode<'g> {
    /// An IRI focus.
    Iri(TypedNode<'g, IriFocus>),
    /// A blank-node focus.
    Blank(TypedNode<'g, BlankFocus>),
    /// A literal focus.
    Literal(TypedNode<'g, LiteralFocus>),
    /// An RDF 1.2 triple-term focus.
    Triple(TypedNode<'g, TripleFocus>),
}

impl<'g> AnyNode<'g> {
    /// Returns the coarse kind of the wrapped focus term.
    pub fn kind(&self) -> TermKind {
        match self {
            Self::Iri(node) => node.kind(),
            Self::Blank(node) => node.kind(),
            Self::Literal(node) => node.kind(),
            Self::Triple(node) => node.kind(),
        }
    }

    /// Returns the focus as a generic [`Term`].
    pub fn term(&self) -> Term {
        match self {
            Self::Iri(node) => node.term(),
            Self::Blank(node) => node.term(),
            Self::Literal(node) => node.term(),
            Self::Triple(node) => node.term(),
        }
    }

    /// Erases the focus kind, yielding the untyped [`Node`] wrapper.
    pub fn into_node(self) -> Node<'g> {
        match self {
            Self::Iri(node) => node.into_node(),
            Self::Blank(node) => node.into_node(),
            Self::Literal(node) => node.into_node(),
            Self::Triple(node) => node.into_node(),
        }
    }
}
