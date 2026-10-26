#![doc = include_str!("../README.md")]

// sq-1rg2q M1: a small, opt-in object view over sparq-core's real Graph.

pub mod proposed;

use oxrdf::vocab::{rdf, xsd};
use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use std::fmt;

/// An owned or borrowed sparq graph exposed through the object-wrapper API.
///
/// `Store::borrowed` never takes ownership and cannot be mutated through the
/// wrapper. `Store::owned` owns its graph and enables mutation. Nodes always
/// borrow the store for their lifetime; Rust therefore requires callers to drop
/// or stop using nodes before mutating an owned store, then reacquire them.
pub struct Store<'g> {
    storage: Storage<'g>,
}

enum Storage<'g> {
    Borrowed(&'g Graph),
    Owned(Box<Graph>),
}

impl<'g> Store<'g> {
    /// Wraps a graph without taking ownership.
    pub fn borrowed(graph: &'g Graph) -> Self {
        Self {
            storage: Storage::Borrowed(graph),
        }
    }

    /// Returns the wrapped graph.
    pub fn graph(&self) -> &Graph {
        match &self.storage {
            Storage::Borrowed(graph) => graph,
            Storage::Owned(graph) => graph,
        }
    }

    /// Returns a borrowed dataset view over the default graph.
    pub fn dataset(&self) -> Dataset<'_> {
        Dataset {
            graph: self.graph(),
        }
    }

    /// Wraps a focus term. The term may be absent from the graph; traversal then
    /// produces an empty iterator, while its typed term accessors still work.
    pub fn node(&self, focus: impl Into<Term>) -> Node<'_> {
        Node {
            graph: self.graph(),
            focus: focus.into(),
        }
    }

    /// Returns the owned graph for mutation, or a typed error for a borrowed store.
    pub fn graph_mut(&mut self) -> Result<&mut Graph, StoreError> {
        match &mut self.storage {
            Storage::Owned(graph) => Ok(graph),
            Storage::Borrowed(_) => Err(StoreError::Borrowed),
        }
    }

    /// Inserts one default-graph triple into an owned store.
    pub fn insert(
        &mut self,
        subject: impl Into<Term>,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<(), StoreError> {
        let subject = subject.into();
        if subject.is_literal() {
            return Err(StoreError::LiteralSubject);
        }
        self.graph_mut()?
            .insert_triple(subject, predicate, object.into())
            .map_err(StoreError::Graph)
    }

    /// Removes one default-graph triple from an owned store.
    pub fn remove(
        &mut self,
        subject: impl Into<Term>,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<(), StoreError> {
        let subject = subject.into();
        if subject.is_literal() {
            return Err(StoreError::LiteralSubject);
        }
        self.graph_mut()?
            .remove_triple(subject, predicate, object.into())
            .map_err(StoreError::Graph)
    }

    /// Consumes the wrapper and returns its graph, or errors if it was borrowed.
    pub fn into_graph(self) -> Result<Graph, StoreError> {
        match self.storage {
            Storage::Owned(graph) => Ok(*graph),
            Storage::Borrowed(_) => Err(StoreError::Borrowed),
        }
    }
}

impl Store<'static> {
    /// Owns an existing graph.
    pub fn owned(graph: Graph) -> Self {
        Self {
            storage: Storage::Owned(Box::new(graph)),
        }
    }

    /// Creates an empty owned store.
    pub fn new() -> Self {
        Self::owned(Graph::new())
    }
}

impl Default for Store<'static> {
    fn default() -> Self {
        Self::new()
    }
}

/// A read-only default-graph view returned by `Node::dataset` and `Store::dataset`.
#[derive(Clone, Copy)]
pub struct Dataset<'g> {
    graph: &'g Graph,
}

impl<'g> Dataset<'g> {
    /// Returns the underlying sparq graph.
    pub fn graph(self) -> &'g Graph {
        self.graph
    }

    /// Wraps a focus term against this dataset.
    pub fn node(self, focus: impl Into<Term>) -> Node<'g> {
        Node {
            graph: self.graph,
            focus: focus.into(),
        }
    }

    /// **PROPOSED**: returns each matching subject exactly once.
    ///
    /// This implements the still-unlanded distinct dataset-helper proposal from
    /// rdfjs/wrapper issue #25 and draft PR #88:
    /// <https://github.com/rdfjs/wrapper/issues/25> and
    /// <https://github.com/rdfjs/wrapper/pull/88>.
    #[cfg(feature = "proposed-distinct")]
    pub fn subjects_of(self, predicate: &NamedNode) -> NodeSet<'g> {
        self.distinct_position(predicate, 0)
    }

    /// **PROPOSED**: returns each matching object exactly once.
    ///
    /// This is the object-side twin of the still-unlanded rdfjs/wrapper distinct
    /// dataset-helper proposal: <https://github.com/rdfjs/wrapper/pull/88>.
    #[cfg(feature = "proposed-distinct")]
    pub fn objects_of(self, predicate: &NamedNode) -> NodeSet<'g> {
        self.distinct_position(predicate, 2)
    }

    #[cfg(feature = "proposed-distinct")]
    fn distinct_position(self, predicate: &NamedNode, position: usize) -> NodeSet<'g> {
        let Some(predicate) = self.graph.id_of(&Term::NamedNode(predicate.clone())) else {
            return NodeSet::empty(self.graph);
        };
        let scan = self.graph.store().scan(&[None, Some(predicate), None]);
        let mut seen = std::collections::HashSet::new();
        let mut terms = Vec::new();
        for row in scan.rows.iter() {
            let id = scan.to_spo(row)[position];
            let term = self.graph.dict().term(id);
            if seen.insert(term.clone()) {
                terms.push(term);
            }
        }
        NodeSet::new(self.graph, terms)
    }
}

/// A focus term bound to a borrowed sparq graph.
#[derive(Clone)]
pub struct Node<'g> {
    graph: &'g Graph,
    focus: Term,
}

impl fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("focus", &self.focus)
            .finish_non_exhaustive()
    }
}

impl<'g> Node<'g> {
    /// Returns the wrapped focus term.
    pub fn focus(&self) -> &Term {
        &self.focus
    }

    /// Unwraps this node into its owned focus term.
    pub fn into_term(self) -> Term {
        self.focus
    }

    /// Returns this node's dataset escape hatch.
    pub fn dataset(&self) -> Dataset<'g> {
        Dataset { graph: self.graph }
    }

    /// Traverses `(focus, predicate, object)` and returns an iterator of matched objects.
    pub fn out(&self, predicate: &NamedNode) -> NodeSet<'g> {
        self.traverse(predicate, Direction::Out)
    }

    /// Traverses `(subject, predicate, focus)` and returns an iterator of matched subjects.
    pub fn r#in(&self, predicate: &NamedNode) -> NodeSet<'g> {
        self.traverse(predicate, Direction::In)
    }

    /// Returns a one-element iterator that removes the wrapper and yields the focus term.
    pub fn values(&self) -> std::iter::Once<Term> {
        std::iter::once(self.focus.clone())
    }

    /// Returns the lexical form of an `xsd:string` or language-tagged string literal.
    pub fn as_str(&self) -> Result<&str, AccessError> {
        let literal = self.literal()?;
        let datatype = literal.datatype();
        if datatype == xsd::STRING || datatype == rdf::LANG_STRING {
            Ok(literal.value())
        } else {
            Err(AccessError::Datatype {
                expected: "xsd:string or rdf:langString",
                found: datatype.into_owned(),
            })
        }
    }

    /// Parses an XML Schema integer-family literal as `i64`.
    pub fn as_i64(&self) -> Result<i64, AccessError> {
        let literal = self.literal()?;
        let datatype = literal.datatype();
        if !is_i64_datatype(datatype.as_str()) {
            return Err(AccessError::Datatype {
                expected: "an XML Schema integer datatype",
                found: datatype.into_owned(),
            });
        }
        let value = literal
            .value()
            .parse::<i128>()
            .map_err(|_| invalid_lexical(literal))?;
        if !integer_facet_accepts(datatype.as_str(), value) {
            return Err(invalid_lexical(literal));
        }
        i64::try_from(value).map_err(|_| invalid_lexical(literal))
    }

    /// Parses an `xsd:boolean` literal (`true`, `false`, `1`, or `0`).
    pub fn as_bool(&self) -> Result<bool, AccessError> {
        let literal = self.literal()?;
        let datatype = literal.datatype();
        if datatype != xsd::BOOLEAN {
            return Err(AccessError::Datatype {
                expected: "xsd:boolean",
                found: datatype.into_owned(),
            });
        }
        match literal.value() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(invalid_lexical(literal)),
        }
    }

    /// Returns the literal's lexical form, datatype, and optional language tag.
    pub fn as_typed_literal(&self) -> Result<TypedLiteral<'_>, AccessError> {
        let literal = self.literal()?;
        Ok(TypedLiteral {
            lexical_form: literal.value(),
            datatype: literal.datatype(),
            language: literal.language(),
        })
    }

    /// **PROPOSED**: requires exactly one outgoing value or returns a typed error.
    ///
    /// This follows the still-unlanded typed-cardinality-error proposal in
    /// rdfjs/wrapper draft PR #89: <https://github.com/rdfjs/wrapper/pull/89>.
    #[cfg(feature = "proposed-cardinality")]
    pub fn required_out(&self, predicate: &NamedNode) -> Result<Node<'g>, CardinalityError> {
        self.singular_out(predicate, Cardinality::ExactlyOne)
            .map(|node| node.expect("exactly-one cardinality returns a node"))
    }

    /// **PROPOSED**: returns zero or one outgoing value, with a typed error for many.
    ///
    /// This follows the still-unlanded typed-cardinality-error proposal in
    /// rdfjs/wrapper draft PR #89: <https://github.com/rdfjs/wrapper/pull/89>.
    #[cfg(feature = "proposed-cardinality")]
    pub fn optional_out(
        &self,
        predicate: &NamedNode,
    ) -> Result<Option<Node<'g>>, CardinalityError> {
        self.singular_out(predicate, Cardinality::AtMostOne)
    }

    #[cfg(feature = "proposed-cardinality")]
    fn singular_out(
        &self,
        predicate: &NamedNode,
        expected: Cardinality,
    ) -> Result<Option<Node<'g>>, CardinalityError> {
        let nodes = self.out(predicate);
        let found = nodes.len();
        let valid = match expected {
            Cardinality::ExactlyOne => found == 1,
            Cardinality::AtMostOne => found <= 1,
        };
        if !valid {
            return Err(CardinalityError {
                focus: self.focus.clone(),
                predicate: predicate.clone(),
                expected,
                found,
            });
        }
        Ok(nodes.into_iter().next())
    }

    fn literal(&self) -> Result<&Literal, AccessError> {
        match &self.focus {
            Term::Literal(literal) => Ok(literal),
            other => Err(AccessError::ExpectedLiteral {
                found: TermKind::of(other),
            }),
        }
    }

    fn traverse(&self, predicate: &NamedNode, direction: Direction) -> NodeSet<'g> {
        let Some(focus) = self.graph.id_of(&self.focus) else {
            return NodeSet::empty(self.graph);
        };
        let Some(predicate) = self.graph.id_of(&Term::NamedNode(predicate.clone())) else {
            return NodeSet::empty(self.graph);
        };
        let pattern = match direction {
            Direction::Out => [Some(focus), Some(predicate), None],
            Direction::In => [None, Some(predicate), Some(focus)],
        };
        let position = match direction {
            Direction::Out => 2,
            Direction::In => 0,
        };
        let scan = self.graph.store().scan(&pattern);
        let terms = scan
            .rows
            .iter()
            .map(|row| self.graph.dict().term(scan.to_spo(row)[position]))
            .collect();
        NodeSet::new(self.graph, terms)
    }
}

enum Direction {
    Out,
    In,
}

/// An exact-size iterator of wrapped RDF nodes.
pub struct NodeSet<'g> {
    graph: &'g Graph,
    terms: std::vec::IntoIter<Term>,
}

impl<'g> NodeSet<'g> {
    fn new(graph: &'g Graph, terms: Vec<Term>) -> Self {
        Self {
            graph,
            terms: terms.into_iter(),
        }
    }

    fn empty(graph: &'g Graph) -> Self {
        Self::new(graph, Vec::new())
    }

    /// Removes the wrappers and yields the matched RDF terms.
    pub fn values(self) -> Values<'g> {
        Values { nodes: self }
    }
}

impl<'g> Iterator for NodeSet<'g> {
    type Item = Node<'g>;

    fn next(&mut self) -> Option<Self::Item> {
        self.terms.next().map(|focus| Node {
            graph: self.graph,
            focus,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.terms.size_hint()
    }
}

impl ExactSizeIterator for NodeSet<'_> {}

/// An iterator that removes node wrappers and yields RDF terms.
pub struct Values<'g> {
    nodes: NodeSet<'g>,
}

impl Iterator for Values<'_> {
    type Item = Term;

    fn next(&mut self) -> Option<Self::Item> {
        self.nodes.next().map(Node::into_term)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.nodes.size_hint()
    }
}

impl ExactSizeIterator for Values<'_> {}

/// A borrowed typed-literal view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypedLiteral<'a> {
    /// The literal's lexical form.
    pub lexical_form: &'a str,
    /// The literal's datatype IRI.
    pub datatype: oxrdf::NamedNodeRef<'a>,
    /// The literal's language tag, if present.
    pub language: Option<&'a str>,
}

/// A coarse RDF term kind used in typed access errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKind {
    /// An IRI.
    NamedNode,
    /// A blank node.
    BlankNode,
    /// A literal.
    Literal,
    /// An RDF 1.2 triple term.
    Triple,
}

impl TermKind {
    fn of(term: &Term) -> Self {
        match term {
            Term::NamedNode(_) => Self::NamedNode,
            Term::BlankNode(_) => Self::BlankNode,
            Term::Literal(_) => Self::Literal,
            Term::Triple(_) => Self::Triple,
        }
    }
}

impl fmt::Display for TermKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::NamedNode => "named node",
            Self::BlankNode => "blank node",
            Self::Literal => "literal",
            Self::Triple => "triple term",
        };
        f.write_str(name)
    }
}

/// Errors from typed RDF value accessors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessError {
    /// The focus term was not a literal.
    ExpectedLiteral { found: TermKind },
    /// The literal's datatype does not support the requested accessor.
    Datatype {
        expected: &'static str,
        found: NamedNode,
    },
    /// The datatype matched, but its lexical form could not be converted.
    InvalidLexical { datatype: NamedNode, value: String },
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpectedLiteral { found } => write!(f, "expected literal, found {found}"),
            Self::Datatype { expected, found } => {
                write!(
                    f,
                    "expected {expected}, found datatype <{}>",
                    found.as_str()
                )
            }
            Self::InvalidLexical { datatype, value } => {
                write!(
                    f,
                    "invalid lexical form {value:?} for <{}>",
                    datatype.as_str()
                )
            }
        }
    }
}

impl std::error::Error for AccessError {}

/// Errors from store ownership or graph mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// Mutation or ownership transfer was attempted through a borrowed store.
    Borrowed,
    /// RDF literals cannot be triple subjects.
    LiteralSubject,
    /// The underlying graph rejected the mutation.
    Graph(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Borrowed => f.write_str("operation requires an owned store"),
            Self::LiteralSubject => f.write_str("an RDF literal cannot be a triple subject"),
            Self::Graph(message) => write!(f, "graph mutation failed: {message}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// The singular-result contract expected by a proposed accessor.
#[cfg(feature = "proposed-cardinality")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    /// Exactly one result.
    ExactlyOne,
    /// Zero or one result.
    AtMostOne,
}

/// A typed error from a proposed singular traversal accessor.
#[cfg(feature = "proposed-cardinality")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardinalityError {
    /// The node from which traversal started.
    pub focus: Term,
    /// The traversed predicate.
    pub predicate: NamedNode,
    /// The expected result arity.
    pub expected: Cardinality,
    /// The number of values found.
    pub found: usize,
}

#[cfg(feature = "proposed-cardinality")]
impl fmt::Display for CardinalityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let expected = match self.expected {
            Cardinality::ExactlyOne => "exactly one",
            Cardinality::AtMostOne => "at most one",
        };
        write!(
            f,
            "expected {expected} value for {} from {}, found {}",
            self.predicate, self.focus, self.found
        )
    }
}

#[cfg(feature = "proposed-cardinality")]
impl std::error::Error for CardinalityError {}

fn is_i64_datatype(datatype: &str) -> bool {
    matches!(
        datatype,
        "http://www.w3.org/2001/XMLSchema#integer"
            | "http://www.w3.org/2001/XMLSchema#long"
            | "http://www.w3.org/2001/XMLSchema#int"
            | "http://www.w3.org/2001/XMLSchema#short"
            | "http://www.w3.org/2001/XMLSchema#byte"
            | "http://www.w3.org/2001/XMLSchema#nonPositiveInteger"
            | "http://www.w3.org/2001/XMLSchema#negativeInteger"
            | "http://www.w3.org/2001/XMLSchema#nonNegativeInteger"
            | "http://www.w3.org/2001/XMLSchema#positiveInteger"
            | "http://www.w3.org/2001/XMLSchema#unsignedLong"
            | "http://www.w3.org/2001/XMLSchema#unsignedInt"
            | "http://www.w3.org/2001/XMLSchema#unsignedShort"
            | "http://www.w3.org/2001/XMLSchema#unsignedByte"
    )
}

fn integer_facet_accepts(datatype: &str, value: i128) -> bool {
    match datatype {
        "http://www.w3.org/2001/XMLSchema#long" => {
            (i128::from(i64::MIN)..=i128::from(i64::MAX)).contains(&value)
        }
        "http://www.w3.org/2001/XMLSchema#int" => {
            (i128::from(i32::MIN)..=i128::from(i32::MAX)).contains(&value)
        }
        "http://www.w3.org/2001/XMLSchema#short" => {
            (i128::from(i16::MIN)..=i128::from(i16::MAX)).contains(&value)
        }
        "http://www.w3.org/2001/XMLSchema#byte" => {
            (i128::from(i8::MIN)..=i128::from(i8::MAX)).contains(&value)
        }
        "http://www.w3.org/2001/XMLSchema#nonPositiveInteger" => value <= 0,
        "http://www.w3.org/2001/XMLSchema#negativeInteger" => value < 0,
        "http://www.w3.org/2001/XMLSchema#nonNegativeInteger" => value >= 0,
        "http://www.w3.org/2001/XMLSchema#positiveInteger" => value > 0,
        "http://www.w3.org/2001/XMLSchema#unsignedLong" => {
            (0..=i128::from(u64::MAX)).contains(&value)
        }
        "http://www.w3.org/2001/XMLSchema#unsignedInt" => {
            (0..=i128::from(u32::MAX)).contains(&value)
        }
        "http://www.w3.org/2001/XMLSchema#unsignedShort" => {
            (0..=i128::from(u16::MAX)).contains(&value)
        }
        "http://www.w3.org/2001/XMLSchema#unsignedByte" => {
            (0..=i128::from(u8::MAX)).contains(&value)
        }
        _ => true,
    }
}

fn invalid_lexical(literal: &Literal) -> AccessError {
    AccessError::InvalidLexical {
        datatype: literal.datatype().into_owned(),
        value: literal.value().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::integer_facet_accepts;

    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

    fn assert_exact_bounds(local: &str, min: i128, max: i128) {
        let datatype = format!("{XSD}{local}");
        assert!(integer_facet_accepts(&datatype, min), "{local} minimum");
        assert!(integer_facet_accepts(&datatype, max), "{local} maximum");
        assert!(
            !integer_facet_accepts(&datatype, min - 1),
            "{local} accepts one below minimum"
        );
        assert!(
            !integer_facet_accepts(&datatype, max + 1),
            "{local} accepts one above maximum"
        );
    }

    #[test]
    fn bounded_integer_datatypes_enforce_exact_min_and_max() {
        assert_exact_bounds("long", i128::from(i64::MIN), i128::from(i64::MAX));
        assert_exact_bounds("int", i128::from(i32::MIN), i128::from(i32::MAX));
        assert_exact_bounds("short", i128::from(i16::MIN), i128::from(i16::MAX));
        assert_exact_bounds("byte", i128::from(i8::MIN), i128::from(i8::MAX));
        assert_exact_bounds("unsignedLong", 0, i128::from(u64::MAX));
        assert_exact_bounds("unsignedInt", 0, i128::from(u32::MAX));
        assert_exact_bounds("unsignedShort", 0, i128::from(u16::MAX));
        assert_exact_bounds("unsignedByte", 0, i128::from(u8::MAX));

        let accepts = |local, value| integer_facet_accepts(&format!("{XSD}{local}"), value);
        assert!(accepts("nonPositiveInteger", 0));
        assert!(!accepts("nonPositiveInteger", 1));
        assert!(accepts("negativeInteger", -1));
        assert!(!accepts("negativeInteger", 0));
        assert!(accepts("nonNegativeInteger", 0));
        assert!(!accepts("nonNegativeInteger", -1));
        assert!(accepts("positiveInteger", 1));
        assert!(!accepts("positiveInteger", 0));
    }
}
