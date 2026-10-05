//! Effective-change subscriptions for an owned wrapper graph.
//!
//! [`ObservableStore`] is a self-contained proposal corresponding to the
//! still-unlanded rdfjs/wrapper dataset and live-value event work in draft PRs
//! #93 and #94. Every effective default-graph mutation emits one synchronous
//! typed event; set-valued no-ops stay silent. Observers run after the graph
//! mutation completes and may therefore inspect the committed graph safely.

// [GPT-5.6] sq-1rg2q.5: effective dataset and live-value change subscriptions.

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use std::fmt;

/// The kind of effective graph mutation reported to an observer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// A triple was newly added.
    Add,
    /// A present triple was deleted.
    Delete,
}

/// One effective default-graph mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeEvent {
    /// Whether the triple was added or deleted.
    pub kind: ChangeKind,
    /// The triple subject.
    pub subject: Term,
    /// The triple predicate.
    pub predicate: NamedNode,
    /// The triple object.
    pub object: Term,
}

/// A live-value event projected from a matching [`ChangeEvent`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueChange<T> {
    /// Whether the matching value was added or deleted.
    pub kind: ChangeKind,
    /// The mapped application value.
    pub value: T,
}

/// Opaque handle returned by a dataset or live-value subscription.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

/// An error from an observed graph mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObserveError {
    /// RDF literals cannot occupy the subject position.
    LiteralSubject,
    /// The backing graph rejected the mutation.
    Graph(String),
}

impl fmt::Display for ObserveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LiteralSubject => f.write_str("RDF literals cannot be triple subjects"),
            Self::Graph(message) => write!(f, "graph mutation failed: {message}"),
        }
    }
}

impl std::error::Error for ObserveError {}

impl ChangeKind {
    /// Whether this mutation changes a graph in which the triple's presence is `present`.
    pub(crate) fn is_effective(self, present: bool) -> bool {
        match self {
            Self::Add => !present,
            Self::Delete => present,
        }
    }
}

/// An ordered subscriber registry shared by every observed wrapper surface.
// [FABLE-5] sq-1rg2q.7/.9/.10: the async and graph-scope event surfaces reuse this.
pub(crate) struct Subscribers<F: ?Sized> {
    entries: Vec<(SubscriptionId, Box<F>)>,
    next: u64,
}

impl<F: ?Sized> Subscribers<F> {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
            next: 0,
        }
    }

    pub(crate) fn add(&mut self, subscriber: Box<F>) -> SubscriptionId {
        let id = SubscriptionId(self.next);
        self.next = self
            .next
            .checked_add(1)
            .expect("subscription identifier space exhausted");
        self.entries.push((id, subscriber));
        id
    }

    pub(crate) fn remove(&mut self, id: SubscriptionId) -> bool {
        let old_len = self.entries.len();
        self.entries.retain(|(candidate, _)| *candidate != id);
        self.entries.len() != old_len
    }

    /// Subscribers in subscription order.
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut F> {
        self.entries
            .iter_mut()
            .map(|(_, subscriber)| &mut **subscriber)
    }
}

type Observer = dyn FnMut(&ChangeEvent, &Graph);

/// An owned graph with synchronous effective-change subscriptions.
///
/// Dataset observers are called in subscription order. The graph mutation is
/// complete before dispatch begins, so the `&Graph` passed to each observer is
/// the committed post-mutation view. Calling [`unsubscribe`](Self::unsubscribe)
/// with an unknown or already-removed handle is a no-op.
pub struct ObservableStore {
    graph: Graph,
    observers: Subscribers<Observer>,
}

impl ObservableStore {
    /// Creates an empty observed store.
    pub fn new() -> Self {
        Self::from_graph(Graph::new())
    }

    /// Wraps an owned graph without copying its triples.
    pub fn from_graph(graph: Graph) -> Self {
        Self {
            graph,
            observers: Subscribers::new(),
        }
    }

    /// Returns the committed graph view.
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Consumes this wrapper and returns its graph.
    pub fn into_graph(self) -> Graph {
        self.graph
    }

    /// Subscribes to every effective mutation.
    ///
    /// The callback receives both the owned event data and the committed graph
    /// view. It is never invoked for a duplicate add or an absent delete.
    pub fn subscribe(
        &mut self,
        observer: impl FnMut(&ChangeEvent, &Graph) + 'static,
    ) -> SubscriptionId {
        self.observers.add(Box::new(observer))
    }

    /// Removes a subscription and reports whether it was present.
    pub fn unsubscribe(&mut self, id: SubscriptionId) -> bool {
        self.observers.remove(id)
    }

    /// Returns a mutation view bound to one RDF subject.
    pub fn node(&mut self, subject: impl Into<Term>) -> ObservableNode<'_> {
        ObservableNode {
            store: self,
            subject: subject.into(),
        }
    }

    /// Returns a live view over the objects of `(subject, predicate, ?object)`.
    pub fn live_values(
        &mut self,
        subject: impl Into<Term>,
        predicate: NamedNode,
    ) -> LiveValues<'_> {
        LiveValues {
            store: self,
            subject: subject.into(),
            predicate,
        }
    }

    fn change(
        &mut self,
        kind: ChangeKind,
        subject: Term,
        predicate: NamedNode,
        object: Term,
    ) -> Result<bool, ObserveError> {
        if subject.is_literal() {
            return Err(ObserveError::LiteralSubject);
        }

        if !kind.is_effective(contains(&self.graph, &subject, &predicate, &object)) {
            return Ok(false);
        }

        // The mutable graph borrow ends here. Event callbacks run only after it
        // and observe the committed graph state.
        apply(&mut self.graph, kind, &subject, &predicate, &object)?;

        let event = ChangeEvent {
            kind,
            subject,
            predicate,
            object,
        };
        let graph = &self.graph;
        for observer in self.observers.iter_mut() {
            observer(&event, graph);
        }
        Ok(true)
    }
}

impl Default for ObservableStore {
    fn default() -> Self {
        Self::new()
    }
}

/// A subject-bound mutation view over an [`ObservableStore`].
pub struct ObservableNode<'store> {
    store: &'store mut ObservableStore,
    subject: Term,
}

impl ObservableNode<'_> {
    /// Adds one outgoing triple and reports whether the graph changed.
    pub fn insert(
        &mut self,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<bool, ObserveError> {
        self.store.change(
            ChangeKind::Add,
            self.subject.clone(),
            predicate,
            object.into(),
        )
    }

    /// Deletes one outgoing triple and reports whether the graph changed.
    pub fn remove(
        &mut self,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<bool, ObserveError> {
        self.store.change(
            ChangeKind::Delete,
            self.subject.clone(),
            predicate,
            object.into(),
        )
    }
}

/// A live object collection backed by one subject and predicate.
///
/// Reads re-query the graph. Mutations share the store's effective-change path,
/// and mapped subscriptions filter dataset events to this collection.
pub struct LiveValues<'store> {
    store: &'store mut ObservableStore,
    subject: Term,
    predicate: NamedNode,
}

impl LiveValues<'_> {
    /// Returns the collection's current RDF object terms.
    pub fn values(&self) -> Vec<Term> {
        let graph = &self.store.graph;
        let Some(pattern) = graph.pattern(Some(&self.subject), Some(&self.predicate), None) else {
            return Vec::new();
        };
        let scan = graph.store.scan(&pattern);
        scan.rows
            .iter()
            .map(|row| graph.dict.term(scan.to_spo(row)[2]))
            .collect()
    }

    /// Inserts an RDF object term and reports whether the graph changed.
    pub fn insert(&mut self, object: impl Into<Term>) -> Result<bool, ObserveError> {
        self.store.change(
            ChangeKind::Add,
            self.subject.clone(),
            self.predicate.clone(),
            object.into(),
        )
    }

    /// Deletes an RDF object term and reports whether the graph changed.
    pub fn remove(&mut self, object: impl Into<Term>) -> Result<bool, ObserveError> {
        self.store.change(
            ChangeKind::Delete,
            self.subject.clone(),
            self.predicate.clone(),
            object.into(),
        )
    }

    /// Subscribes to mapped changes for this subject and predicate only.
    ///
    /// `map` projects the event's RDF object into an application value before
    /// `observer` runs. The returned handle may be detached through this live
    /// view or directly through [`ObservableStore::unsubscribe`].
    pub fn subscribe<T: 'static>(
        &mut self,
        map: impl Fn(&Term) -> T + 'static,
        mut observer: impl FnMut(&ValueChange<T>, &Graph) + 'static,
    ) -> SubscriptionId {
        let subject = self.subject.clone();
        let predicate = self.predicate.clone();
        self.store.subscribe(move |event, graph| {
            if event.subject == subject && event.predicate == predicate {
                observer(
                    &ValueChange {
                        kind: event.kind,
                        value: map(&event.object),
                    },
                    graph,
                );
            }
        })
    }

    /// Removes a dataset or mapped-value subscription.
    pub fn unsubscribe(&mut self, id: SubscriptionId) -> bool {
        self.store.unsubscribe(id)
    }
}

pub(crate) fn contains(
    graph: &Graph,
    subject: &Term,
    predicate: &NamedNode,
    object: &Term,
) -> bool {
    graph
        .pattern(Some(subject), Some(predicate), Some(object))
        .is_some_and(|pattern| !graph.store.scan(&pattern).rows.is_empty())
}

/// Applies one mutation to `graph`, which the caller has already found effective.
pub(crate) fn apply(
    graph: &mut Graph,
    kind: ChangeKind,
    subject: &Term,
    predicate: &NamedNode,
    object: &Term,
) -> Result<(), ObserveError> {
    let (s, p, o) = (subject.clone(), predicate.clone(), object.clone());
    match kind {
        ChangeKind::Add => graph.insert_triple(s, p, o),
        ChangeKind::Delete => graph.remove_triple(s, p, o),
    }
    .map_err(ObserveError::Graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::Literal;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn iri(local: &str) -> NamedNode {
        NamedNode::new(format!("http://example.org/{local}")).unwrap()
    }

    fn literal(value: &str) -> Term {
        Literal::new_simple_literal(value).into()
    }

    #[test]
    fn dataset_subscription_reports_only_effective_changes_from_both_paths() {
        let mut store = ObservableStore::new();
        let alice = iri("alice");
        let name = iri("name");
        let tag = iri("tag");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_by_callback = Rc::clone(&seen);
        let subscription = store.subscribe(move |event, graph| {
            let event_is_present = contains(graph, &event.subject, &event.predicate, &event.object);
            seen_by_callback
                .borrow_mut()
                .push((event.clone(), graph.len(), event_is_present));
        });

        {
            let mut node = store.node(alice.clone());
            assert!(node.insert(name.clone(), literal("Alice")).unwrap());
            assert!(!node.insert(name.clone(), literal("Alice")).unwrap());
            assert!(!node.remove(name.clone(), literal("missing")).unwrap());
        }
        {
            let mut tags = store.live_values(alice.clone(), tag.clone());
            assert!(tags.insert(literal("rust")).unwrap());
            assert!(!tags.insert(literal("rust")).unwrap());
            assert!(!tags.remove(literal("missing")).unwrap());
            assert!(tags.remove(literal("rust")).unwrap());
            assert!(tags.values().is_empty());
        }
        {
            let mut node = store.node(alice.clone());
            assert!(node.remove(name.clone(), literal("Alice")).unwrap());
        }

        let events = seen.borrow();
        assert_eq!(
            events.iter().map(|(event, _, _)| event).collect::<Vec<_>>(),
            vec![
                &ChangeEvent {
                    kind: ChangeKind::Add,
                    subject: alice.clone().into(),
                    predicate: name.clone(),
                    object: literal("Alice"),
                },
                &ChangeEvent {
                    kind: ChangeKind::Add,
                    subject: alice.clone().into(),
                    predicate: tag.clone(),
                    object: literal("rust"),
                },
                &ChangeEvent {
                    kind: ChangeKind::Delete,
                    subject: alice.clone().into(),
                    predicate: tag,
                    object: literal("rust"),
                },
                &ChangeEvent {
                    kind: ChangeKind::Delete,
                    subject: alice.clone().into(),
                    predicate: name.clone(),
                    object: literal("Alice"),
                },
            ]
        );
        assert_eq!(
            events
                .iter()
                .map(|(_, graph_len, present)| (*graph_len, *present))
                .collect::<Vec<_>>(),
            vec![(1, true), (2, true), (1, false), (0, false)]
        );
        drop(events);

        assert!(store.unsubscribe(subscription));
        assert!(!store.unsubscribe(subscription));
        store.node(alice).insert(name, literal("silent")).unwrap();
        assert_eq!(seen.borrow().len(), 4);
    }

    #[test]
    fn live_subscription_filters_and_maps_values_across_fresh_views() {
        let mut store = ObservableStore::new();
        let alice = iri("alice");
        let tag = iri("tag");
        let other = iri("other");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_by_callback = Rc::clone(&seen);

        let subscription = store.live_values(alice.clone(), tag.clone()).subscribe(
            |term| match term {
                Term::Literal(literal) => literal.value().to_owned(),
                _ => panic!("test subscription expects a literal"),
            },
            move |event, graph| {
                seen_by_callback
                    .borrow_mut()
                    .push((event.clone(), graph.len()));
            },
        );

        store
            .node(alice.clone())
            .insert(tag.clone(), literal("one"))
            .unwrap();
        store
            .node(alice.clone())
            .insert(other, literal("filtered"))
            .unwrap();
        store
            .live_values(alice.clone(), tag.clone())
            .remove(literal("one"))
            .unwrap();

        assert_eq!(
            *seen.borrow(),
            vec![
                (
                    ValueChange {
                        kind: ChangeKind::Add,
                        value: "one".to_owned(),
                    },
                    1,
                ),
                (
                    ValueChange {
                        kind: ChangeKind::Delete,
                        value: "one".to_owned(),
                    },
                    1,
                ),
            ]
        );

        assert!(store
            .live_values(alice.clone(), tag.clone())
            .unsubscribe(subscription));
        store
            .live_values(alice, tag)
            .insert(literal("silent"))
            .unwrap();
        assert_eq!(seen.borrow().len(), 2);
    }

    #[test]
    fn literal_subject_is_rejected_without_mutation_or_event() {
        let mut store = ObservableStore::new();
        let calls = Rc::new(RefCell::new(0));
        let calls_by_callback = Rc::clone(&calls);
        store.subscribe(move |_, _| *calls_by_callback.borrow_mut() += 1);

        let result = store
            .node(literal("not a subject"))
            .insert(iri("predicate"), literal("object"));

        assert_eq!(result, Err(ObserveError::LiteralSubject));
        assert!(store.graph().is_empty());
        assert_eq!(*calls.borrow(), 0);
    }
}
