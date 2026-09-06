//! Cycle-safe JSON projection for wrapped RDF objects.
//!
//! [`JsonProjection`] renders a focus node, and everything reachable from it
//! through outgoing predicates, as one JSON document. This is the Rust wrapper
//! slice of the still-unlanded rdfjs/wrapper JSON-mapping proposal in open PR
//! #23: <https://github.com/rdfjs/wrapper/pull/23>. The document shape below is
//! this crate's own; it is not a line-by-line port of that pull request.
//!
//! Two properties are load-bearing, because an RDF graph is neither a tree nor
//! acyclic:
//!
//! * **Total.** A cycle is never followed twice. When expansion reaches a node
//!   the projection is already rendering, it emits a reference instead of
//!   recursing, so `a -> b -> a` terminates. Which repeats become references is
//!   an explicit choice, not an accident of traversal order: see
//!   [`RepeatedFocus`]. [`JsonProjection::with_max_depth`] additionally bounds
//!   recursion depth on very deep acyclic chains.
//! * **Deterministic.** Predicates are emitted in lexicographic IRI order and
//!   each predicate's objects in lexicographic N-Triples order, so projecting
//!   the same store twice produces byte-identical output.
//!
//! Literal metadata survives the projection: a typed literal keeps its
//! datatype IRI and a language-tagged literal keeps its language tag (and RDF
//! 1.2 base direction). Nothing is coerced to a bare JSON string, number, or
//! boolean, so no lexical form and no datatype is lost on the way out.

// [SONNET-4.6] sq-1rg2q.11: deterministic, cycle-total JSON projection.

use crate::Node;
use oxrdf::{Literal, Term};
use sparq_core::Graph;
use std::collections::{BTreeMap, HashSet};

/// The node-expansion depth a [`JsonProjection`] uses unless told otherwise.
///
/// Expansion recurses, so an unbounded depth would let a pathologically long
/// chain of nodes exhaust the stack. Beyond this depth a node is emitted as a
/// reference, exactly as a repeat is.
pub const DEFAULT_MAX_DEPTH: usize = 64;

/// What a projection emits when expansion reaches a node it has met before.
///
/// A reference is the JSON object `{"@ref": "<term>"}`, where the term is the
/// same stable identifier the expanded form carries in its `@id`. It is
/// deliberately a distinct key from `@id`, so a reference to a node is never
/// confused with a node that happens to have no outgoing predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatedFocus {
    /// Reference a node only while it is an ancestor of the node being written.
    ///
    /// This is the minimum needed to stay total: it breaks cycles and nothing
    /// else. A node reachable by two different paths — a diamond — is expanded
    /// once per path, so the document repeats that subtree.
    OnCycle,
    /// Reference any node already expanded anywhere earlier in the document.
    ///
    /// Every node is expanded at most once, which also collapses diamonds and
    /// keeps the document linear in the size of the reachable subgraph. Which
    /// occurrence wins is fixed by the projection's own sorted traversal order,
    /// so this stays deterministic.
    OnRepeat,
}

/// A configured, reusable JSON projection over wrapped RDF objects.
///
/// # Examples
///
/// ```
/// use oxrdf::NamedNode;
/// use sparq_core::Graph;
/// use sparq_wrapper::proposed::json::JsonProjection;
/// use sparq_wrapper::Store;
///
/// let graph = Graph::load_str(
///     "@prefix ex: <http://example.org/> . ex:a ex:knows ex:b . ex:b ex:knows ex:a .",
///     "turtle",
/// )?;
/// let store = Store::borrowed(&graph);
/// let a = NamedNode::new("http://example.org/a")?;
///
/// let json = JsonProjection::new().project(&store.node(a));
/// // The cycle back to `a` is a reference, so the projection terminates.
/// assert!(json.contains(r#"{"@ref":"http://example.org/a"}"#));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonProjection {
    repeated_focus: RepeatedFocus,
    max_depth: usize,
}

impl JsonProjection {
    /// Creates a projection that references cycles only, to
    /// [`DEFAULT_MAX_DEPTH`].
    pub fn new() -> Self {
        Self {
            repeated_focus: RepeatedFocus::OnCycle,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }

    /// Selects which repeated nodes become references.
    pub fn with_repeated_focus(mut self, repeated_focus: RepeatedFocus) -> Self {
        self.repeated_focus = repeated_focus;
        self
    }

    /// Bounds how deeply nodes are expanded before becoming references.
    ///
    /// The bound applies to expandable terms only, so a depth of `0` projects a
    /// non-literal focus as a reference but still projects a literal focus as
    /// its value object — a literal has no outgoing edges, so there is no
    /// recursion there to bound. The bound is a stack guard, not a policy for
    /// repeats — cycles are already broken by [`RepeatedFocus`] at any depth.
    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Projects `node` and its outgoing reachable subgraph as compact JSON.
    ///
    /// Only outgoing predicates are followed, and only in the default graph —
    /// the same surface [`Node::out`](crate::Node::out) traverses. A focus term
    /// absent from the graph projects to a node object with no predicates; a
    /// literal focus projects to its value object.
    pub fn project(&self, node: &Node<'_>) -> String {
        let mut projector = Projector {
            projection: self,
            graph: node.dataset().graph(),
            path: Vec::new(),
            expanded: HashSet::new(),
            out: String::new(),
        };
        projector.write_term(node.focus(), 0);
        projector.out
    }
}

impl Default for JsonProjection {
    fn default() -> Self {
        Self::new()
    }
}

/// The mutable state of one in-progress projection.
struct Projector<'a> {
    projection: &'a JsonProjection,
    graph: &'a Graph,
    /// The ancestor chain of the node currently being written, for
    /// [`RepeatedFocus::OnCycle`].
    path: Vec<Term>,
    /// Every node expanded so far, for [`RepeatedFocus::OnRepeat`].
    expanded: HashSet<Term>,
    out: String,
}

impl Projector<'_> {
    fn write_term(&mut self, term: &Term, depth: usize) {
        // A literal is a leaf: it has no outgoing edges, so neither the depth
        // bound nor the repeat policy has any recursion here to cut, and it is
        // written as a value object at every depth.
        if let Term::Literal(literal) = term {
            self.write_literal(literal);
            return;
        }
        // Short-circuit order matters: a node cut off by the depth bound was not
        // expanded, so `OnRepeat` must not record it as if it had been.
        if depth >= self.projection.max_depth || self.is_repeat(term) {
            self.write_reference(term);
            return;
        }
        self.path.push(term.clone());
        self.write_node(term, depth);
        self.path.pop();
    }

    fn is_repeat(&mut self, term: &Term) -> bool {
        match self.projection.repeated_focus {
            RepeatedFocus::OnCycle => self.path.contains(term),
            RepeatedFocus::OnRepeat => !self.expanded.insert(term.clone()),
        }
    }

    fn write_node(&mut self, term: &Term, depth: usize) {
        self.out.push_str("{\"@id\":");
        write_json_string(&term_id(term), &mut self.out);
        for (predicate, objects) in outgoing(self.graph, term) {
            self.out.push(',');
            write_json_string(&predicate, &mut self.out);
            self.out.push_str(":[");
            for (index, object) in objects.iter().enumerate() {
                if index > 0 {
                    self.out.push(',');
                }
                self.write_term(object, depth + 1);
            }
            self.out.push(']');
        }
        self.out.push('}');
    }

    fn write_reference(&mut self, term: &Term) {
        self.out.push_str("{\"@ref\":");
        write_json_string(&term_id(term), &mut self.out);
        self.out.push('}');
    }

    /// Writes a literal as a value object that keeps every component of the RDF
    /// literal. A language tag implies `rdf:langString` (with a base direction,
    /// `rdf:dirLangString`), so the datatype is emitted only when there is no
    /// tag to imply it; no case drops a lexical form, datatype, tag, or
    /// direction.
    fn write_literal(&mut self, literal: &Literal) {
        self.out.push_str("{\"@value\":");
        write_json_string(literal.value(), &mut self.out);
        match literal.language() {
            Some(language) => {
                self.out.push_str(",\"@language\":");
                write_json_string(language, &mut self.out);
                if let Some(direction) = literal.direction() {
                    self.out.push_str(",\"@direction\":");
                    write_json_string(&direction.to_string(), &mut self.out);
                }
            }
            None => {
                self.out.push_str(",\"@type\":");
                write_json_string(literal.datatype().as_str(), &mut self.out);
            }
        }
        self.out.push('}');
    }
}

/// The stable identifier a node is written and referenced by.
///
/// An IRI is its own bare string; every other term uses its N-Triples form,
/// which is injective, so two distinct terms never share an identifier.
fn term_id(term: &Term) -> String {
    match term {
        Term::NamedNode(node) => node.as_str().to_owned(),
        other => other.to_string(),
    }
}

/// Collects a focus term's outgoing default-graph edges in deterministic order:
/// predicates sorted by IRI (the `BTreeMap`), objects sorted by N-Triples form.
fn outgoing(graph: &Graph, focus: &Term) -> BTreeMap<String, Vec<Term>> {
    let mut edges: BTreeMap<String, Vec<Term>> = BTreeMap::new();
    let Some(focus) = graph.id_of(focus) else {
        return edges;
    };
    let scan = graph.store.scan(&[Some(focus), None, None]);
    for row in scan.rows.iter() {
        let triple = scan.to_spo(row);
        edges
            .entry(term_id(&graph.dict.term(triple[1])))
            .or_default()
            .push(graph.dict.term(triple[2]));
    }
    for objects in edges.values_mut() {
        objects.sort_by_cached_key(ToString::to_string);
        objects.dedup();
    }
    edges
}

/// Appends `value` as a quoted, escaped JSON string.
fn write_json_string(value: &str, out: &mut String) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            // Every other C0 control is illegal unescaped in a JSON string.
            control if control < '\u{20}' => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::{JsonProjection, RepeatedFocus};
    use crate::Store;
    use oxrdf::vocab::xsd;
    use oxrdf::{BaseDirection, Literal, NamedNode};
    use sparq_core::Graph;

    fn iri(local: &str) -> NamedNode {
        NamedNode::new(format!("http://example.org/{}", local)).expect("valid IRI")
    }

    /// `a -> b`, `a -> c`, and both `b` and `c` point at the same `d`.
    fn diamond() -> Graph {
        Graph::load_str(
            "@prefix ex: <http://example.org/> .\n\
             ex:a ex:to ex:b, ex:c .\n\
             ex:b ex:to ex:d .\n\
             ex:c ex:to ex:d .\n",
            "turtle",
        )
        .expect("valid turtle")
    }

    #[test]
    fn new_and_default_reference_cycles_only() {
        let graph = diamond();
        let store = Store::borrowed(&graph);
        let node = store.node(iri("a"));

        let projected = JsonProjection::new().project(&node);
        assert_eq!(projected, JsonProjection::default().project(&node));
        // `OnCycle` is the default, so the shared `d` is expanded under both
        // paths rather than referenced. Flipping the default to `OnRepeat`
        // turns the second `d` into `{"@ref":...}` and fails this.
        assert_eq!(
            projected
                .matches(r#""@id":"http://example.org/d""#)
                .count(),
            2
        );
        assert!(!projected.contains("@ref"));
    }

    #[test]
    fn with_repeated_focus_collapses_a_shared_node_to_one_expansion() {
        let graph = diamond();
        let store = Store::borrowed(&graph);

        let projected = JsonProjection::new()
            .with_repeated_focus(RepeatedFocus::OnRepeat)
            .project(&store.node(iri("a")));

        assert_eq!(
            projected
                .matches(r#""@id":"http://example.org/d""#)
                .count(),
            1
        );
        assert!(projected.contains(r#"{"@ref":"http://example.org/d"}"#));
    }

    #[test]
    fn with_max_depth_truncates_expansion_to_references() {
        let graph = diamond();
        let store = Store::borrowed(&graph);
        let node = store.node(iri("a"));

        assert_eq!(
            JsonProjection::new().with_max_depth(0).project(&node),
            r#"{"@ref":"http://example.org/a"}"#
        );
        assert_eq!(
            JsonProjection::new().with_max_depth(1).project(&node),
            "{\"@id\":\"http://example.org/a\",\"http://example.org/to\":\
             [{\"@ref\":\"http://example.org/b\"},{\"@ref\":\"http://example.org/c\"}]}"
        );
    }

    #[test]
    fn max_depth_zero_still_projects_a_literal_focus_as_its_value_object() {
        let store = Store::new();
        let focus = Literal::new_typed_literal("42", xsd::INTEGER);

        // The depth bound guards recursion, and a literal has none to guard, so
        // it keeps its value object where a node focus becomes `{"@ref":...}`.
        assert_eq!(
            JsonProjection::new()
                .with_max_depth(0)
                .project(&store.node(focus)),
            r#"{"@value":"42","@type":"http://www.w3.org/2001/XMLSchema#integer"}"#
        );
    }

    #[test]
    fn project_escapes_control_characters_and_quotes_in_lexical_forms() {
        let mut store = Store::new();
        let subject = iri("subject");
        let predicate = iri("predicate");
        store
            .insert(
                subject.clone(),
                predicate,
                Literal::new_simple_literal("a \"quote\",\n\tan\u{1}escape"),
            )
            .expect("insert");

        let projected = JsonProjection::new().project(&store.node(subject));

        assert!(
            projected.contains(r#""@value":"a \"quote\",\n\tan\u0001escape""#),
            "escaped lexical form missing from {}",
            projected
        );
    }

    #[test]
    fn a_focus_absent_from_the_graph_projects_to_a_node_with_no_predicates() {
        let graph = diamond();
        let store = Store::borrowed(&graph);

        assert_eq!(
            JsonProjection::new().project(&store.node(iri("absent"))),
            r#"{"@id":"http://example.org/absent"}"#
        );
    }

    #[test]
    fn a_directional_language_literal_keeps_its_tag_and_direction() {
        let store = Store::new();
        let focus = Literal::new_directional_language_tagged_literal(
            "الويب الدلالي",
            "ar",
            BaseDirection::Rtl,
        )
        .expect("valid directional literal");

        assert_eq!(
            JsonProjection::new().project(&store.node(focus)),
            r#"{"@value":"الويب الدلالي","@language":"ar","@direction":"rtl"}"#
        );
    }

    #[test]
    fn a_literal_focus_projects_to_its_value_object() {
        let store = Store::new();
        let focus = Literal::new_typed_literal("42", xsd::INTEGER);

        assert_eq!(
            JsonProjection::new().project(&store.node(focus)),
            r#"{"@value":"42","@type":"http://www.w3.org/2001/XMLSchema#integer"}"#
        );
    }
}
