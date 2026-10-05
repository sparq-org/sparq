//! Projected effective-change events for graph-scoped views.
//!
//! [`ObservableDataset`] owns a dataset and reports quad mutations through the
//! read [`Projection`] of a graph scope. This is the Rust wrapper slice of the
//! still-unlanded rdfjs/wrapper projected-event proposal in draft PR #96.
//!
//! A scope reads the deduplicated union of its graphs, so a projected listener
//! sees the *union* change: an add only when the first in-scope copy of a
//! triple appears, a delete only when the last in-scope copy disappears, and
//! nothing for a mutation of a graph outside the projection.

// [FABLE-5] sq-1rg2q.7: union-boundary events over the observe.rs machinery.

use super::graph_scope::Projection;
use super::observe::{
    apply, contains, ChangeEvent, ChangeKind, ObserveError, Subscribers, SubscriptionId,
};
use oxrdf::{NamedNode, Term};
use sparq_core::Graph;

type Dispatch = dyn FnMut(Option<&Term>, &ChangeEvent, &Graph);

/// An owned dataset with projected effective-change subscriptions.
///
/// Mutations name their target graph (`None` is the default graph). Each
/// subscription carries its own [`Projection`]; listeners run in subscription
/// order after the mutation is committed and receive the committed dataset.
pub struct ObservableDataset {
    dataset: Graph,
    observers: Subscribers<Dispatch>,
}

impl ObservableDataset {
    /// Creates an empty observed dataset.
    pub fn new() -> Self {
        Self::from_graph(Graph::new())
    }

    /// Wraps an owned dataset without copying its quads.
    pub fn from_graph(dataset: Graph) -> Self {
        Self {
            dataset,
            observers: Subscribers::new(),
        }
    }

    /// Returns the committed dataset view.
    pub fn graph(&self) -> &Graph {
        &self.dataset
    }

    /// Consumes this wrapper and returns its dataset.
    pub fn into_graph(self) -> Graph {
        self.dataset
    }

    /// Subscribes to union-boundary changes seen through `projection`.
    ///
    /// The callback receives the event, the configured projection, and the
    /// committed dataset. Pass `scope.projection().clone()` to follow an
    /// existing [`GraphScope`](super::graph_scope::GraphScope).
    pub fn subscribe(
        &mut self,
        projection: Projection,
        mut observer: impl FnMut(&ChangeEvent, &Projection, &Graph) + 'static,
    ) -> SubscriptionId {
        self.observers
            .add(Box::new(move |graph_name, event, dataset| {
                if !projection.includes(graph_name) {
                    return;
                }
                let copies = projection
                    .graphs(dataset)
                    .filter(|graph| {
                        contains(graph, &event.subject, &event.predicate, &event.object)
                    })
                    .count();
                let boundary = match event.kind {
                    ChangeKind::Add => copies == 1,
                    ChangeKind::Delete => copies == 0,
                };
                if boundary {
                    observer(event, &projection, dataset);
                }
            }))
    }

    /// Removes a subscription and reports whether it was present.
    pub fn unsubscribe(&mut self, id: SubscriptionId) -> bool {
        self.observers.remove(id)
    }

    /// Inserts one quad and reports whether its graph changed.
    pub fn insert(
        &mut self,
        graph_name: Option<&Term>,
        subject: impl Into<Term>,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<bool, ObserveError> {
        self.change(
            ChangeKind::Add,
            graph_name,
            subject.into(),
            predicate,
            object.into(),
        )
    }

    /// Removes one quad and reports whether its graph changed.
    pub fn remove(
        &mut self,
        graph_name: Option<&Term>,
        subject: impl Into<Term>,
        predicate: NamedNode,
        object: impl Into<Term>,
    ) -> Result<bool, ObserveError> {
        self.change(
            ChangeKind::Delete,
            graph_name,
            subject.into(),
            predicate,
            object.into(),
        )
    }

    fn change(
        &mut self,
        kind: ChangeKind,
        graph_name: Option<&Term>,
        subject: Term,
        predicate: NamedNode,
        object: Term,
    ) -> Result<bool, ObserveError> {
        if subject.is_literal() {
            return Err(ObserveError::LiteralSubject);
        }
        let target = match graph_name {
            None => &mut self.dataset,
            // An absent graph holds nothing to delete; do not create it.
            Some(name)
                if kind == ChangeKind::Delete && self.dataset.named_graph(name).is_none() =>
            {
                return Ok(false)
            }
            Some(name) => {
                let index = self
                    .dataset
                    .ensure_named(name)
                    .map_err(ObserveError::Graph)?;
                &mut self.dataset.named[index].1
            }
        };
        if !kind.is_effective(contains(target, &subject, &predicate, &object)) {
            return Ok(false);
        }
        apply(target, kind, &subject, &predicate, &object)?;

        let event = ChangeEvent {
            kind,
            subject,
            predicate,
            object,
        };
        for observer in self.observers.iter_mut() {
            observer(graph_name, &event, &self.dataset);
        }
        Ok(true)
    }
}

impl Default for ObservableDataset {
    fn default() -> Self {
        Self::new()
    }
}
