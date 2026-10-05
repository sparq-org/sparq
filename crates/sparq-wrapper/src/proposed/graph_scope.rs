//! Read-many/write-one graph-scoped wrapper views.
//!
//! [`GraphScope`] projects an explicit set of readable named graphs (and,
//! optionally, the default graph) into one deduplicated traversal surface.
//! Every mutation is routed to one configured named graph. This is the Rust
//! wrapper slice of the still-unlanded rdfjs/wrapper graph-scope proposal in
//! draft PR #95.

// [GPT-5.6] sq-1rg2q.6: explicit read projection with a single write target.

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;

/// A view that reads an explicit graph set and writes one named graph.
///
/// `readable_named_graphs` is exact: absent graph names contribute no triples,
/// and unlisted graphs are invisible. The default graph is excluded unless
/// [`with_default_graph`](Self::with_default_graph) is called. Repeating a
/// readable graph name or storing the same triple in multiple readable graphs
/// does not duplicate traversal results.
///
/// The view temporarily borrows the dataset mutably so nodes may write through
/// it. Traversals materialize their deduplicated term results before returning,
/// so a later node mutation never overlaps a borrow of the underlying graph.
pub struct GraphScope<'graph> {
    graph: RefCell<&'graph mut Graph>,
    projection: Projection,
    write_graph: Term,
}

impl fmt::Debug for GraphScope<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphScope")
            .field("projection", &self.projection)
            .field("write_graph", &self.write_graph)
            .finish_non_exhaustive()
    }
}

/// The exact set of graphs a [`GraphScope`] reads, independent of any dataset.
///
/// Repeated graph names are kept once, in first-seen order. The default graph
/// is excluded unless [`with_default_graph`](Self::with_default_graph) is called.
// sq-1rg2q.7: shared by scoped reads and projected change events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    readable_named_graphs: Vec<Term>,
    read_default_graph: bool,
}

impl Projection {
    /// Creates a projection over exactly the supplied named graphs.
    pub fn new<I, G>(readable_named_graphs: I) -> Self
    where
        I: IntoIterator<Item = G>,
        G: Into<Term>,
    {
        let mut seen = HashSet::new();
        let readable_named_graphs = readable_named_graphs
            .into_iter()
            .map(Into::into)
            .filter(|name| seen.insert(name.clone()))
            .collect();
        Self {
            readable_named_graphs,
            read_default_graph: false,
        }
    }

    /// Includes the dataset's default graph in the projection.
    pub fn with_default_graph(mut self) -> Self {
        self.read_default_graph = true;
        self
    }

    /// Returns the readable named-graph terms in traversal order.
    pub fn readable_named_graphs(&self) -> &[Term] {
        &self.readable_named_graphs
    }

    /// Returns whether the default graph is part of the projection.
    pub fn reads_default_graph(&self) -> bool {
        self.read_default_graph
    }

    /// Returns whether `graph_name` (`None` for the default graph) is projected.
    pub fn includes(&self, graph_name: Option<&Term>) -> bool {
        match graph_name {
            None => self.read_default_graph,
            Some(name) => self.readable_named_graphs.contains(name),
        }
    }

    /// The projected graphs that exist in `dataset`, default graph first.
    pub(crate) fn graphs<'d>(&'d self, dataset: &'d Graph) -> impl Iterator<Item = &'d Graph> {
        self.read_default_graph
            .then_some(dataset)
            .into_iter()
            .chain(
                self.readable_named_graphs
                    .iter()
                    .filter_map(|name| dataset.named_graph(name)),
            )
    }
}

impl<'graph> GraphScope<'graph> {
    /// Creates a graph scope whose reads come from exactly the supplied named graphs.
    ///
    /// The default graph is initially excluded. Call
    /// [`with_default_graph`](Self::with_default_graph) to include it in the
    /// read projection. The `write_graph` does not need to be readable and is
    /// created on the first insert or remove when it does not yet exist. It
    /// must be an IRI or a blank node; with any other term every insert and
    /// remove fails with [`GraphScopeError::InvalidGraphName`].
    pub fn new<I, G>(
        graph: &'graph mut Graph,
        readable_named_graphs: I,
        write_graph: impl Into<Term>,
    ) -> Self
    where
        I: IntoIterator<Item = G>,
        G: Into<Term>,
    {
        Self {
            graph: RefCell::new(graph),
            projection: Projection::new(readable_named_graphs),
            write_graph: write_graph.into(),
        }
    }

    /// Includes the dataset's default graph in the readable projection.
    pub fn with_default_graph(mut self) -> Self {
        self.projection = self.projection.with_default_graph();
        self
    }

    /// Returns the configured read projection.
    pub fn projection(&self) -> &Projection {
        &self.projection
    }

    /// Returns the configured readable named-graph terms in traversal order.
    pub fn readable_named_graphs(&self) -> &[Term] {
        self.projection.readable_named_graphs()
    }

    /// Returns whether the default graph is part of the readable projection.
    pub fn reads_default_graph(&self) -> bool {
        self.projection.reads_default_graph()
    }

    /// Returns the sole named-graph term targeted by writes and deletes.
    pub fn write_graph(&self) -> &Term {
        &self.write_graph
    }

    /// Wraps a focus term against this graph scope.
    pub fn node(&self, focus: impl Into<Term>) -> Node<'_, 'graph> {
        Node {
            scope: self,
            focus: focus.into(),
        }
    }

    /// Inserts one triple into only the configured write graph.
    pub fn insert(
        &mut self,
        subject: impl Into<Term>,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<(), GraphScopeError> {
        self.change(Change::Insert, subject.into(), predicate, object.into())
    }

    /// Removes one triple from only the configured write graph.
    ///
    /// Copies of the triple in the default graph or other named graphs remain
    /// untouched, including other graphs in the readable projection.
    pub fn remove(
        &mut self,
        subject: impl Into<Term>,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<(), GraphScopeError> {
        self.change(Change::Remove, subject.into(), predicate, object.into())
    }

    fn change(
        &self,
        change: Change,
        subject: Term,
        predicate: NamedNode,
        object: Term,
    ) -> Result<(), GraphScopeError> {
        if subject.is_literal() {
            return Err(GraphScopeError::LiteralSubject);
        }
        if !is_graph_name(&self.write_graph) {
            return Err(GraphScopeError::InvalidGraphName(self.write_graph.clone()));
        }

        let mut dataset = self.graph.borrow_mut();
        let dataset = &mut **dataset;
        let index = dataset
            .ensure_named(&self.write_graph)
            .map_err(GraphScopeError::Graph)?;
        let write_graph = &mut dataset.named[index].1;
        match change {
            Change::Insert => write_graph.insert_triple(subject, predicate, object),
            Change::Remove => write_graph.remove_triple(subject, predicate, object),
        }
        .map_err(GraphScopeError::Graph)
    }

    fn traverse(&self, focus: &Term, predicate: &NamedNode, direction: Direction) -> Vec<Term> {
        let dataset = self.graph.borrow();
        let dataset = &**dataset;
        let mut seen = HashSet::new();
        let mut terms = Vec::new();
        for graph in self.projection.graphs(dataset) {
            append_traversal(graph, focus, predicate, direction, &mut seen, &mut terms);
        }
        terms
    }
}

/// A focus term bound to a [`GraphScope`].
///
/// Both this node and nodes returned from its traversals retain the same scope,
/// so chained `out`/`in` reads use the identical deduplicated graph projection.
/// Calling [`insert`](Self::insert) or [`remove`](Self::remove) always targets
/// the scope's one write graph.
#[derive(Clone)]
pub struct Node<'scope, 'graph> {
    scope: &'scope GraphScope<'graph>,
    focus: Term,
}

impl fmt::Debug for Node<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("focus", &self.focus)
            .field("write_graph", &self.scope.write_graph)
            .finish_non_exhaustive()
    }
}

impl<'scope, 'graph> Node<'scope, 'graph> {
    /// Returns the wrapped focus term.
    pub fn focus(&self) -> &Term {
        &self.focus
    }

    /// Unwraps this node into its owned focus term.
    pub fn into_term(self) -> Term {
        self.focus
    }

    /// Traverses `(focus, predicate, object)` across the readable projection.
    pub fn out(&self, predicate: &NamedNode) -> NodeSet<'scope, 'graph> {
        NodeSet::new(
            self.scope,
            self.scope.traverse(&self.focus, predicate, Direction::Out),
        )
    }

    /// Traverses `(subject, predicate, focus)` across the readable projection.
    pub fn r#in(&self, predicate: &NamedNode) -> NodeSet<'scope, 'graph> {
        NodeSet::new(
            self.scope,
            self.scope.traverse(&self.focus, predicate, Direction::In),
        )
    }

    /// Returns a one-element iterator containing the focus term.
    pub fn values(&self) -> std::iter::Once<Term> {
        std::iter::once(self.focus.clone())
    }

    /// Inserts one outgoing triple into only the scope's write graph.
    pub fn insert(
        &self,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<(), GraphScopeError> {
        self.scope
            .change(Change::Insert, self.focus.clone(), predicate, object.into())
    }

    /// Removes one outgoing triple from only the scope's write graph.
    pub fn remove(
        &self,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<(), GraphScopeError> {
        self.scope
            .change(Change::Remove, self.focus.clone(), predicate, object.into())
    }
}

/// An exact-size iterator of nodes bound to one graph scope.
pub struct NodeSet<'scope, 'graph> {
    scope: &'scope GraphScope<'graph>,
    terms: std::vec::IntoIter<Term>,
}

impl<'scope, 'graph> NodeSet<'scope, 'graph> {
    fn new(scope: &'scope GraphScope<'graph>, terms: Vec<Term>) -> Self {
        Self {
            scope,
            terms: terms.into_iter(),
        }
    }

    /// Removes the wrappers and yields the matched RDF terms.
    pub fn values(self) -> Values<'scope, 'graph> {
        Values { nodes: self }
    }
}

impl<'scope, 'graph> Iterator for NodeSet<'scope, 'graph> {
    type Item = Node<'scope, 'graph>;

    fn next(&mut self) -> Option<Self::Item> {
        self.terms.next().map(|focus| Node {
            scope: self.scope,
            focus,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.terms.size_hint()
    }
}

impl ExactSizeIterator for NodeSet<'_, '_> {}

/// An iterator that removes scoped node wrappers and yields RDF terms.
pub struct Values<'scope, 'graph> {
    nodes: NodeSet<'scope, 'graph>,
}

impl Iterator for Values<'_, '_> {
    type Item = Term;

    fn next(&mut self) -> Option<Self::Item> {
        self.nodes.next().map(Node::into_term)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.nodes.size_hint()
    }
}

impl ExactSizeIterator for Values<'_, '_> {}

/// An error from a graph-scoped mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphScopeError {
    /// RDF literals cannot occupy the subject position.
    LiteralSubject,
    /// A named graph can only be named by an IRI or a blank node.
    InvalidGraphName(Term),
    /// The backing named graph rejected the mutation.
    Graph(String),
}

impl fmt::Display for GraphScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LiteralSubject => f.write_str("RDF literals cannot be triple subjects"),
            Self::InvalidGraphName(name) => write!(f, "{name} cannot name a graph"),
            Self::Graph(message) => write!(f, "graph mutation failed: {message}"),
        }
    }
}

impl std::error::Error for GraphScopeError {}

/// Whether `name` may name a graph: an IRI or a blank node, never a literal
/// or a triple term.
///
/// Checked before any write creates a named graph, because a graph created
/// under any other term cannot be persisted and reopened.
pub(crate) fn is_graph_name(name: &Term) -> bool {
    matches!(name, Term::NamedNode(_) | Term::BlankNode(_))
}

#[derive(Clone, Copy)]
enum Direction {
    Out,
    In,
}

#[derive(Clone, Copy)]
enum Change {
    Insert,
    Remove,
}

fn append_traversal(
    graph: &Graph,
    focus: &Term,
    predicate: &NamedNode,
    direction: Direction,
    seen: &mut HashSet<Term>,
    terms: &mut Vec<Term>,
) {
    let Some(focus) = graph.id_of(focus) else {
        return;
    };
    let Some(predicate) = graph.id_of(&Term::NamedNode(predicate.clone())) else {
        return;
    };
    let (pattern, position) = match direction {
        Direction::Out => ([Some(focus), Some(predicate), None], 2),
        Direction::In => ([None, Some(predicate), Some(focus)], 0),
    };
    let scan = graph.store.scan(&pattern);
    for row in scan.rows.iter() {
        let term = graph.dict.term(scan.to_spo(row)[position]);
        if seen.insert(term.clone()) {
            terms.push(term);
        }
    }
}
